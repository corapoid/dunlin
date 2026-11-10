//! Supported relay endpoint naming. Network membership and ACLs are enforced
//! by Tailscale; a hostname is not itself proof that Funnel is disabled.
pub fn tailscale_hostname(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host.ends_with(".ts.net")
        && host.split('.').count() >= 3
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_tailscale_dns_names_are_supported() {
        assert!(tailscale_hostname("build.tail123.ts.net"));
        for host in [
            "ts.net",
            "build.ts.net.evil.test",
            "127.0.0.1",
            "build.example.com",
            ".ts.net",
        ] {
            assert!(!tailscale_hostname(host));
        }
    }
}
