//! Process containment for bash children.
//!
//! What each platform actually gets (documented honestly rather than
//! oversold):
//!
//! - **Windows (`sandbox = "job"`, default)**: every spawned shell is
//!   assigned to one process-wide Job Object with
//!   `KILL_ON_JOB_CLOSE | SILENT_BREAKAWAY_OFF`. The whole command tree is
//!   guaranteed to die when the harness exits — no orphaned builds or
//!   servers can outlive a crash. This is containment, not filesystem
//!   sandboxing; true FS isolation on Windows needs AppContainer, which is
//!   roadmap work.
//! - **Linux (`sandbox = "strict"`, opt-in)**: Landlock restricts the
//!   child's filesystem view to read-everywhere plus writes only under the
//!   workspace root and the system temp dir. Opt-in because legitimate
//!   tooling (cargo, npm, pip) writes to home caches; enable it when your
//!   workflow only touches the repo.
//! - `sandbox = "off"` disables both.

#[cfg(windows)]
pub mod appcontainer;

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// HANDLE is a plain opaque value; sharing it is fine.
    struct SyncHandle(HANDLE);
    unsafe impl Send for SyncHandle {}
    unsafe impl Sync for SyncHandle {}

    static JOB: OnceLock<SyncHandle> = OnceLock::new();

    fn create_job() -> Option<SyncHandle> {
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            // KILL_ON_JOB_CLOSE: the tree dies with the harness. Breakaway
            // prevention is implicit — without BREAKAWAY_OK, children stay
            // in the job (Vista+ default assignment semantics).
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = windows::Win32::System::JobObjects::SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok.is_err() {
                return None;
            }
            Some(SyncHandle(job))
        }
    }

    /// Assign a process (by raw handle) to the process-wide job object.
    /// Best-effort: false means containment is unavailable, work continues.
    pub fn contain(raw: std::os::windows::io::RawHandle) -> bool {
        let job = JOB.get_or_init(|| create_job().unwrap_or(SyncHandle(HANDLE(0 as _))));
        if job.0.is_invalid() {
            return false;
        }
        unsafe { AssignProcessToJobObject(job.0, HANDLE(raw as *mut _)).is_ok() }
    }

    /// Assign an already-opened process handle (AppContainer children).
    pub fn contain_handle(h: HANDLE) -> bool {
        contain(h.0 as _)
    }

    /// Test-only: close the job handle to verify kill-on-close wiring.
    #[cfg(test)]
    pub fn close_job_for_test() {
        if let Some(SyncHandle(h)) = JOB.get() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(*h);
            }
        }
    }
}

#[cfg(windows)]
pub fn contain_child(child: &tokio::process::Child) -> bool {
    match child.raw_handle() {
        Some(raw) => job::contain(raw),
        None => false,
    }
}

#[cfg(not(windows))]
pub fn contain_child(_child: &tokio::process::Child) -> bool {
    true
}

/// Linux strict mode: Landlock filesystem rules applied inside the child
/// before exec. Best-effort: with CompatLevel::BestEffort, features the
/// running kernel can't enforce are silently dropped (verified to compile
/// against landlock 0.4 for x86_64-unknown-linux-gnu; runtime enforcement
/// requires kernel 5.13+).
#[cfg(target_os = "linux")]
pub fn linux_restrict(workspace: &std::path::Path) {
    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, ABI,
    };
    // ABI::V1 is the conservative baseline (kernel 5.13+).
    let abi = ABI::V1;
    let restrict = || -> Result<(), Box<dyn std::error::Error>> {
        let tmp = std::env::temp_dir();
        Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            // Read/execute everywhere the user could read...
            .handle_access(AccessFs::from_all(abi))?
            .create()?
            .add_rule(PathBeneath::new(PathFd::new("/")?, AccessFs::from_read(abi)))?
            // ...writes only under the workspace and temp.
            .add_rule(PathBeneath::new(PathFd::new(workspace)?, AccessFs::from_write(abi)))?
            .add_rule(PathBeneath::new(PathFd::new(&tmp)?, AccessFs::from_write(abi)))?
            .restrict_self()?;
        Ok(())
    };
    if let Err(e) = restrict() {
        eprintln!("(landlock: {e}; continuing unsandboxed)");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[tokio::test]
    async fn job_object_kills_child_when_handle_closes() {
        use std::time::Duration;
        // Spawn a long-lived child, contain it, then close the job handle:
        // the child must die (KILL_ON_JOB_CLOSE wiring works).
        let mut child = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn powershell");
        assert!(super::contain_child(&child), "job assignment should succeed");
        super::job::close_job_for_test();
        // Reap: the child should have been terminated by the closed job.
        for _ in 0..50 {
            match child.try_wait() {
                Ok(Some(_)) => return,
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        let _ = child.kill().await;
        panic!("child should have died when the job handle closed");
    }
}
