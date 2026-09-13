//! Destination guard for network-egress tools (web_fetch). The permission
//! layer decides *whether* egress may happen; this decides *where* it may
//! go: URLs carrying credentials always deny, and hosts that resolve to
//! non-public addresses deny unless `web_fetch_private_hosts = true` — the
//! SSRF surface a prompt-injected model could otherwise reach (localhost
//! services, RFC1918 ranges, the 169.254.169.254 cloud-metadata range).
//!
//! Redirects are not followed silently, so every hop re-guards: web_fetch
//! builds its client with `redirect(Policy::none)` and hands 3xx Location
//! values back to the model.

use std::net::{IpAddr, ToSocketAddrs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dest {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

/// Parse an absolute http(s) URL into scheme/host/port. Rejects credentials
/// in the authority (a secret in a URL becomes a secret in the transcript).
pub(crate) fn parse_dest(raw: &str) -> Result<Dest, String> {
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| format!("'{raw}' is not an absolute http(s) URL (no scheme)") )?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("unsupported scheme '{scheme}' (only http/https)"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err("the URL has no host".to_string());
    }
    if authority.contains('@') {
        return Err(
            "the URL carries credentials (user:pass@host) — remove them; secrets must not ride URLs"
                .to_string(),
        );
    }
    let default_port: u16 = if scheme == "https" { 443 } else { 80 };
    // [ipv6-literal]:port / host:port / host. Unbracketed IPv6 is illegal in
    // URLs and would mis-parse as host:port — reject it loudly.
    if authority.matches(':').count() > 1 && !authority.starts_with('[') {
        return Err("malformed authority (IPv6 literals must be bracketed, e.g. http://[::1]:8080/)".to_string());
    }
    let (host, port) = if let Some(rest6) = authority.strip_prefix('[') {
        let close = rest6
            .find(']')
            .ok_or_else(|| "malformed IPv6 literal (missing ']')".to_string())?;
        let host = rest6[..close].to_ascii_lowercase();
        let port = rest6[close + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_ascii_lowercase(), p.parse().unwrap_or(default_port)),
            None => (authority.to_ascii_lowercase(), default_port),
        }
    };
    if host.is_empty() {
        return Err("the URL has no host".to_string());
    }
    Ok(Dest { scheme, host, port })
}

/// Public = reachable from the open internet. Loopback, private, link-local
/// (includes cloud metadata), unspecified, CGNAT, documentation and the v6
/// ULA/link-local ranges are all non-public; v4-mapped v6 recurses.
pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // 100.64.0.0/10 CGNAT shared address space
                || (o[0] == 100 && (o[1] & 0xC0) == 0x40))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unicast_link_local()
                || (s[0] & 0xfe00) == 0xfc00) // fc00::/7 unique-local
        }
    }
}

/// Human-facing deny reason when any resolved address is non-public.
pub(crate) fn deny_reason(host: &str, ips: &[IpAddr]) -> Option<String> {
    let bad: Vec<IpAddr> = ips.iter().copied().filter(|ip| !is_public_ip(*ip)).collect();
    if bad.is_empty() {
        return None;
    }
    let why: Vec<String> = bad
        .iter()
        .map(|ip| match ip {
            IpAddr::V4(v4) if v4.is_loopback() => format!("{ip} (loopback)"),
            IpAddr::V4(v4) if v4.is_link_local() => {
                format!("{ip} (link-local — includes the cloud-metadata range)")
            }
            IpAddr::V4(v4) if v4.is_private() => format!("{ip} (private)"),
            IpAddr::V4(v4) if v4.is_unspecified() => format!("{ip} (unspecified)"),
            IpAddr::V4(_) => format!("{ip} (non-public)"),
            IpAddr::V6(v6) if v6.is_loopback() => format!("{ip} (loopback)"),
            IpAddr::V6(v6) if v6.is_unicast_link_local() => format!("{ip} (link-local)"),
            IpAddr::V6(v6) if (v6.segments()[0] & 0xfe00) == 0xfc00 => format!("{ip} (unique-local)"),
            IpAddr::V6(_) => format!("{ip} (non-public)"),
        })
        .collect();
    Some(format!(
        "host '{host}' resolves to {}. Private and local destinations are denied by default \
         (SSRF guard); set web_fetch_private_hosts = true in myharness.toml if this is an \
         internal/dev service you really need",
        why.join(", ")
    ))
}

/// Guard one URL. Literal-IP hosts are checked DNS-free; hostnames resolve
/// (off the async runtime) and every address must be public.
pub async fn guard(raw: &str, allow_private: bool) -> Result<Dest, String> {
    let dest = parse_dest(raw)?;
    if allow_private {
        return Ok(dest);
    }
    if let Ok(ip) = dest.host.parse::<IpAddr>() {
        return match deny_reason(&dest.host, &[ip]) {
            Some(e) => Err(e),
            None => Ok(dest),
        };
    }
    let host = dest.host.clone();
    let port = dest.port;
    let joined = tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|it| it.map(|a| a.ip()).collect::<Vec<_>>())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("DNS lookup task failed: {e}"))?
    .map_err(|e| format!("could not resolve host '{}' ({e}) — check the hostname", dest.host))?;
    if joined.is_empty() {
        return Err(format!("host '{}' resolved to no addresses", dest.host));
    }
    match deny_reason(&dest.host, &joined) {
        Some(e) => Err(e),
        None => Ok(dest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scheme_host_port() {
        let d = parse_dest("https://Example.com/path?q=1").unwrap();
        assert_eq!(d.scheme, "https");
        assert_eq!(d.host, "example.com");
        assert_eq!(d.port, 443);
        let d = parse_dest("http://example.com:8080/x").unwrap();
        assert_eq!(d.port, 8080);
        let d = parse_dest("http://[2001:db8::1]:9000/x").unwrap();
        assert_eq!(d.host, "2001:db8::1");
        assert_eq!(d.port, 9000);
    }

    #[test]
    fn parse_rejects_credentials_and_junk() {
        assert!(parse_dest("https://user:pass@example.com/").unwrap_err().contains("credentials"));
        assert!(parse_dest("ftp://example.com/").unwrap_err().contains("scheme"));
        assert!(parse_dest("example.com/x").unwrap_err().contains("no scheme"));
        assert!(parse_dest("http:///path").unwrap_err().contains("no host"));
        assert!(parse_dest("http://::1/x").unwrap_err().contains("bracketed"));
    }

    #[test]
    fn public_ip_classification() {
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("93.184.216.34".parse().unwrap()));
        for bad in [
            "127.0.0.1", "10.1.2.3", "192.168.0.5", "172.16.1.1", "169.254.169.254",
            "0.0.0.0", "255.255.255.255", "100.64.0.1", "192.0.2.7",
        ] {
            assert!(!is_public_ip(bad.parse().unwrap()), "{bad} must be non-public");
        }
        assert!(is_public_ip("2606:4700::1111".parse().unwrap()));
        for bad in ["::1", "fe80::1", "fc00::1", "fd12::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1"] {
            assert!(!is_public_ip(bad.parse().unwrap()), "{bad} must be non-public");
        }
    }

    #[test]
    fn deny_reason_names_the_host_and_knob() {
        let r = deny_reason("localhost", &["127.0.0.1".parse().unwrap()]).unwrap();
        assert!(r.contains("host 'localhost'"));
        assert!(r.contains("loopback"));
        assert!(r.contains("web_fetch_private_hosts"));
        assert!(deny_reason("ok", &["8.8.8.8".parse().unwrap()]).is_none());
    }

    #[tokio::test]
    async fn guard_blocks_private_literals_without_dns() {
        for url in [
            "http://127.0.0.1:3000/api",
            "http://10.0.0.5/",
            "http://192.168.1.1/admin",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]:8080/",
            "https://0.0.0.0/",
        ] {
            let err = guard(url, false).await.unwrap_err();
            assert!(err.contains("SSRF guard") || err.contains("non-public"), "{url}: {err}");
        }
        // The knob opens private destinations (credentials still deny).
        assert!(guard("http://127.0.0.1:3000/", true).await.is_ok());
        // Public literals pass without touching DNS.
        assert!(guard("http://8.8.8.8/", false).await.is_ok());
        // Hostnames resolve and classify (localhost resolves offline).
        assert!(guard("http://localhost:3000/", false).await.unwrap_err().contains("loopback"));
        // Credentials deny regardless of the knob.
        assert!(guard("https://u:p@example.com/", true).await.unwrap_err().contains("credentials"));
    }
}
