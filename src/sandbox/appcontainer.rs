//! Windows AppContainer sandboxing (`sandbox = "appcontainer"`): the real
//! filesystem isolation Job Objects can't provide.
//!
//! Every sandboxed command runs inside a dedicated AppContainer profile.
//! Inside the container the process can read only what "ALL APPLICATION
//! PACKAGES" ACEs allow (Windows dirs, most of Program Files), write
//! nowhere except the directories we explicitly grant an ACE for (the
//! workspace and the temp dir), and use NO network (no capabilities are
//! requested — cargo fetch & co. will fail loudly, which is the point of a
//! sandbox; use `job`/`strict` modes for workflows that need egress).
//!
//! What this module does per launch:
//! 1. ensure the `myharness-sandbox` profile exists and derive its SID,
//! 2. grant that SID read/write on the requested directories (the ACE
//!    persists for the session — that SID is only usable by processes
//!    launched inside the container, i.e. by us),
//! 3. CreateProcessW with a SECURITY_CAPABILITIES thread attribute and
//!    redirected stdio pipes (std's Command cannot express this),
//! 4. assign the child to the process-wide Job Object so it still dies
//!    with the harness.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows::Win32::Security::{
    AddAccessAllowedAceEx, AddAce, FreeSid, GetAce, InitializeAcl, ACE_HEADER, ACL, ACL_REVISION_DS,
    DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SECURITY_CAPABILITIES,
};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, OPEN_EXISTING};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, STARTUPINFOEXW, STARTF_USESTDHANDLES,
};

use crate::sandbox::job;

const PROFILE: &str = "myharness-sandbox";

/// Directories we have granted the container SID this session (ACEs stay
/// until the harness exits; harmless — see the module docs).
static GRANTED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A launched sandboxed process: pid, pipes as std files, and the process
/// handle for wait/terminate.
pub struct AcProcess {
    pub pid: u32,
    pub process: HANDLE,
    pub stdout: Option<std::fs::File>,
    pub stderr: Option<std::fs::File>,
}

impl Drop for AcProcess {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.process); }
    }
}

// HANDLE is an opaque value; moving it across threads is fine.
unsafe impl Send for AcProcess {}

impl AcProcess {
    /// Non-blocking exit check (poll from the caller's loop).
    pub fn try_wait(&self) -> Option<i32> {
        unsafe {
            if WaitForSingleObject(self.process, 0) == WAIT_OBJECT_0 {
                let mut code: u32 = 0;
                let _ = GetExitCodeProcess(self.process, &mut code);
                Some(code as i32)
            } else {
                None
            }
        }
    }

    pub fn terminate(&self) {
        unsafe {
            let _ = TerminateProcess(self.process, 1);
        }
    }
}

/// Ensure the profile exists, returning its SID (caller frees).
unsafe fn ensure_sid() -> Result<PSID> {
    let name = wide(PROFILE);
    // Existing profile: derive directly; otherwise create it.
    if let Ok(sid) = DeriveAppContainerSidFromAppContainerName(PCWSTR(name.as_ptr())) {
        return Ok(sid);
    }
    let display = wide("myharness sandbox");
    let description = wide("myharness bash tool sandbox");
    CreateAppContainerProfile(
        PCWSTR(name.as_ptr()),
        PCWSTR(display.as_ptr()),
        PCWSTR(description.as_ptr()),
        None,
    )
    .with_context(|| "creating AppContainer profile (it may be in a bad state; delete 'myharness-sandbox' and retry)")?;
    // Derive again so ownership semantics match both paths.
    DeriveAppContainerSidFromAppContainerName(PCWSTR(name.as_ptr()))
        .context("deriving SID after profile creation")
}

/// Grant the container SID read/write/execute on a directory (appending an
/// ACE to the existing DACL — existing permissions are untouched).
unsafe fn grant_dir(sid: PSID, dir: &Path) -> Result<()> {
    let dir_w = wide(&dir.display().to_string());
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sec_desc = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
    let err = GetNamedSecurityInfoW(
        PCWSTR(dir_w.as_ptr()),
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION,
        None,
        None,
        Some(&mut dacl),
        None,
        &mut sec_desc,
    );
    if err.0 != 0 {
        return Err(anyhow!("reading DACL of {}: WIN32 error {}", dir.display(), err.0));
    }

    // FILE_GENERIC_WRITE | FILE_GENERIC_READ | GENERIC_EXECUTE
    const FILE_ALL_FOR_CONTAINER: u32 = 0x0012_01BF;
    let mut new_dacl_bytes = vec![0u8; (*dacl).AclSize as usize + 64];
    let new_dacl = new_dacl_bytes.as_mut_ptr() as *mut ACL;
    InitializeAcl(new_dacl, new_dacl_bytes.len() as u32, ACL_REVISION_DS)
        .context("InitializeAcl")?;
    // Copy existing ACEs so nothing is dropped.
    for i in 0..(*dacl).AceCount as u32 {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        if GetAce(dacl, i, &mut ace).is_ok() {
            let header = &*(ace as *const ACE_HEADER);
            let _ = AddAce(new_dacl, ACL_REVISION_DS, u32::MAX, ace, header.AceSize as u32);
        }
    }
    AddAccessAllowedAceEx(new_dacl, ACL_REVISION_DS, Default::default(), FILE_ALL_FOR_CONTAINER, sid)
        .context("AddAccessAllowedAceEx")?;
    let err = SetNamedSecurityInfoW(
        PCWSTR(dir_w.as_ptr()),
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION,
        None,
        None,
        Some(new_dacl),
        None,
    );
    if !sec_desc.0.is_null() {
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(sec_desc.0));
    }
    if err.0 != 0 {
        return Err(anyhow!(
            "granting container access to {}: WIN32 error {}",
            dir.display(),
            err.0
        ));
    }
    Ok(())
}

/// Quote one argument with Windows command-line semantics.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
            }
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Launch `program args...` inside the AppContainer with cwd and stdio
/// pipes. `write_dirs` get container-writable ACEs (once per session).
pub fn launch(
    program: &str,
    args: &[String],
    cwd: &Path,
    write_dirs: &[PathBuf],
) -> Result<AcProcess> {
    unsafe {
        let sid = ensure_sid()?;
        let result = launch_inner(sid, program, args, cwd, write_dirs);
        let _ = FreeSid(sid);
        result
    }
}

unsafe fn launch_inner(
    sid: PSID,
    program: &str,
    args: &[String],
    cwd: &Path,
    write_dirs: &[PathBuf],
) -> Result<AcProcess> {
    // Grant write access once per directory per session.
    {
        let mut granted = GRANTED.lock().unwrap();
        let list = granted.get_or_insert_with(Vec::new);
        for dir in write_dirs {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if !list.contains(&canonical) {
                grant_dir(sid, &canonical).with_context(|| {
                    format!("granting sandbox write access to {}", canonical.display())
                })?;
                list.push(canonical);
            }
        }
    }

    // Pipes for stdout/stderr + NUL for stdin.
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (stdout_r, stdout_w) = make_pipe(&sa)?;
    let (stderr_r, stderr_w) = make_pipe(&sa)?;
    let nul = open_nul(&sa)?;

    // Attribute list carrying SECURITY_CAPABILITIES.
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut size: usize = 0;
    let _ = InitializeProcThreadAttributeList(LPPROC_THREAD_ATTRIBUTE_LIST(std::ptr::null_mut()), 1, 0, &mut size);
    let mut buffer = vec![0u8; size.max(64)];
    let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr() as *mut _);
    InitializeProcThreadAttributeList(attr_list, 1, 0, &mut size)
        .context("InitializeProcThreadAttributeList")?;
    let attr_ok = UpdateProcThreadAttribute(
        attr_list,
        0,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
        Some(&caps as *const SECURITY_CAPABILITIES as *const core::ffi::c_void),
        std::mem::size_of::<SECURITY_CAPABILITIES>(),
        None,
        None,
    );
    if let Err(e) = attr_ok {
        DeleteProcThreadAttributeList(attr_list);
        return Err(anyhow!("UpdateProcThreadAttribute(SECURITY_CAPABILITIES) failed: {e}"));
    }

    let mut si: STARTUPINFOEXW = std::mem::zeroed();
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = nul;
    si.StartupInfo.hStdOutput = stdout_w;
    si.StartupInfo.hStdError = stderr_w;

    let mut cmdline = quote_arg(program);
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote_arg(a));
    }
    let mut cmdline_w = wide(&cmdline);
    let cwd_w = wide(&cwd.display().to_string());
    let mut pi = PROCESS_INFORMATION::default();
    let created = CreateProcessW(
        None,
        windows::core::PWSTR(cmdline_w.as_mut_ptr()),
        None,
        None,
        true, // inherit handles (the pipes)
        EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
        None,
        PCWSTR(cwd_w.as_ptr()),
        &si.StartupInfo,
        &mut pi,
    );
    // The parent's copies of the write ends and the attribute list go now.
    let _ = CloseHandle(stdout_w);
    let _ = CloseHandle(stderr_w);
    let _ = CloseHandle(nul);
    DeleteProcThreadAttributeList(attr_list);
    if let Err(e) = created {
        return Err(anyhow!(
            "CreateProcessW inside AppContainer '{PROFILE}' failed: {e} (the program may be unusable from containers)"
        ));
    }
    let _ = CloseHandle(pi.hThread);
    let pid = pi.dwProcessId;
    // Keep job containment on top of the container.
    job::contain_handle(pi.hProcess);

    use std::os::windows::io::FromRawHandle;
    Ok(AcProcess {
        pid,
        process: pi.hProcess,
        stdout: Some(std::fs::File::from_raw_handle(stdout_r.0 as _)),
        stderr: Some(std::fs::File::from_raw_handle(stderr_r.0 as _)),
    })
}

unsafe fn make_pipe(sa: &SECURITY_ATTRIBUTES) -> Result<(HANDLE, HANDLE)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    CreatePipe(&mut read, &mut write, Some(sa), 0).context("CreatePipe")?;
    Ok((read, write))
}

unsafe fn open_nul(sa: &SECURITY_ATTRIBUTES) -> Result<HANDLE> {
    let name = wide("NUL");
    CreateFileW(
        PCWSTR(name.as_ptr()),
        windows::Win32::Foundation::GENERIC_READ.0,
        FILE_SHARE_READ,
        Some(sa),
        OPEN_EXISTING,
        Default::default(),
        None,
    )
    .context("opening NUL for stdin")
}

/// Best-effort cleanup of the profile (kept for manual/tests).
pub fn delete_profile() {
    let name = wide(PROFILE);
    unsafe {
        let _ = DeleteAppContainerProfile(PCWSTR(name.as_ptr()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_quote_correctly() {
        assert_eq!(quote_arg("plain"), "plain");
        // Backslash-only args need no quoting (CommandLineToArgvW keeps them).
        assert_eq!(quote_arg("C:\\path\\"), "C:\\path\\");
        assert_eq!(quote_arg("with space"), "\"with space\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        // Trailing backslashes double when the arg gets quoted.
        assert_eq!(quote_arg("out dir\\"), "\"out dir\\\\\"");
    }
}
