//! Host registry metadata. Reported capabilities and policy intersections are
//! data, never proof of network identity, isolation or execution authority.
use crate::{agent::AGENT_HARNESS_IDS, relay::tailscale_hostname};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const HOST_PROTOCOL_VERSION: u16 = 1;
pub const MAX_HOST_DEFINITION_BYTES: usize = 16 * 1024;
pub const MAX_HOST_CAPABILITIES_BYTES: usize = 16 * 1024;
pub const MAX_HOST_PARALLEL: u16 = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDefinition {
    pub name: String,
    /// Stable enrolled identity, not a mutable hostname or a self-attested proof.
    /// Store must forbid changing it on an existing host.
    pub node_id: String,
    pub serve_origin: String,
    pub capability_policy: HostCapabilityPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilityPolicy {
    pub max_parallel: u16,
    pub harnesses: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilities {
    pub protocol_version: u16,
    pub version: String,
    pub os: String,
    pub architecture: String,
    pub harnesses: Vec<String>,
    pub sandbox: SandboxStrength,
    pub max_parallel: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStrength {
    None,
    LinuxBubblewrap,
    MacosSandboxExec,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Pending,
    Claimed,
    Approved,
    Revoked,
}
/// There is deliberately no browser-deserializable verified/ready variant.
/// Actual Serve/node identity verification needs a separate trusted adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkVerification {
    #[default]
    #[serde(rename = "unverified")]
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostValidationError {
    pub path: String,
    pub message: String,
}
fn invalid(path: impl Into<String>, message: &str) -> HostValidationError {
    HostValidationError {
        path: path.into(),
        message: message.into(),
    }
}
fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn parallel(value: u16, path: &str) -> Result<(), HostValidationError> {
    if !(1..=MAX_HOST_PARALLEL).contains(&value) {
        return Err(invalid(path, "Parallel capacity must be between 1 and 32"));
    }
    Ok(())
}
fn harnesses(values: &[String], path: &str) -> Result<(), HostValidationError> {
    let mut unique = BTreeSet::new();
    if values.len() > AGENT_HARNESS_IDS.len()
        || values
            .iter()
            .any(|value| !AGENT_HARNESS_IDS.contains(&value.as_str()) || !unique.insert(value))
    {
        return Err(invalid(path, "Harnesses must be unique fixed harness IDs"));
    }
    Ok(())
}
impl HostDefinition {
    pub fn validate(&self) -> Result<(), HostValidationError> {
        if !text(&self.name, 128) {
            return Err(invalid(
                "name",
                "A nonempty name of at most 128 UTF-8 bytes is required",
            ));
        }
        if self.node_id.is_empty()
            || self.node_id.len() > 128
            || !self
                .node_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid(
                "node_id",
                "A bounded stable ASCII node identity is required",
            ));
        }
        let host = self.serve_origin.strip_prefix("https://").ok_or_else(|| {
            invalid(
                "serve_origin",
                "Use a canonical private Tailscale HTTPS origin",
            )
        })?;
        // A canonical origin has implicit port 443 and no trailing slash. This
        // rejects credentials, arbitrary ports, IPs and URL parser ambiguities
        // without resolving DNS or treating .ts.net as identity verification.
        if host.len() > 253 || host != host.to_ascii_lowercase() || !tailscale_hostname(host) {
            return Err(invalid(
                "serve_origin",
                "Use lowercase https://host.tailnet.ts.net with implicit port 443 and no path, query or credentials",
            ));
        }
        self.capability_policy.validate().map_err(|mut error| {
            error.path = format!("capability_policy.{}", error.path);
            error
        })
    }
}
impl HostCapabilityPolicy {
    pub fn validate(&self) -> Result<(), HostValidationError> {
        parallel(self.max_parallel, "max_parallel")?;
        harnesses(&self.harnesses, "harnesses")
    }
    /// A resource ceiling only. Even a nonempty result grants no host, project,
    /// network, credential or sandbox authority. Reports may exceed policy.
    pub fn effective(&self, report: &HostCapabilities) -> Result<Self, HostValidationError> {
        self.validate()?;
        report.validate()?;
        Ok(Self {
            max_parallel: self.max_parallel.min(report.max_parallel),
            harnesses: self
                .harnesses
                .iter()
                .filter(|id| report.harnesses.contains(id))
                .cloned()
                .collect(),
        })
    }
}
impl HostCapabilities {
    pub fn validate(&self) -> Result<(), HostValidationError> {
        if self.protocol_version != HOST_PROTOCOL_VERSION {
            return Err(invalid(
                "protocol_version",
                "Unsupported worker protocol version",
            ));
        }
        for (path, value, limit) in [
            ("version", self.version.as_str(), 128),
            ("os", self.os.as_str(), 64),
            ("architecture", self.architecture.as_str(), 64),
        ] {
            if !text(value, limit) {
                return Err(invalid(
                    path,
                    "A bounded nonempty value without control characters is required",
                ));
            }
        }
        parallel(self.max_parallel, "max_parallel")?;
        harnesses(&self.harnesses, "harnesses")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> HostDefinition {
        HostDefinition {
            name: "Private worker".into(),
            node_id: "nStableNode-01".into(),
            serve_origin: "https://worker.tail123.ts.net".into(),
            capability_policy: HostCapabilityPolicy {
                max_parallel: 2,
                harnesses: vec!["codex".into(), "opencode".into()],
            },
        }
    }
    fn capabilities() -> HostCapabilities {
        HostCapabilities {
            protocol_version: 1,
            version: "1.0.0+fixture".into(),
            os: "linux".into(),
            architecture: "aarch64".into(),
            harnesses: vec!["claude".into(), "codex".into()],
            sandbox: SandboxStrength::LinuxBubblewrap,
            max_parallel: 16,
        }
    }
    #[test]
    fn canonical_origin_rejects_credentials_paths_ports_lan_ssh_and_parser_ambiguities() {
        definition().validate().unwrap();
        for origin in [
            "http://worker.tail123.ts.net",
            "https://worker.tail123.ts.net:443",
            "https://worker.tail123.ts.net:8443",
            "https://worker.tail123.ts.net/",
            "https://worker.tail123.ts.net/path",
            "https://worker.tail123.ts.net?x=1",
            "https://worker.tail123.ts.net#x",
            "https://user@worker.tail123.ts.net",
            "https://worker.tail123.ts.net@evil.test",
            "https://127.0.0.1",
            "http://localhost",
            "https://100.100.1.1",
            "https://[::1]",
            "https://worker.example.com",
            "https://worker.tail123.ts.net.evil.test",
            "ssh://worker.tail123.ts.net",
            "https://WORKER.tail123.ts.net",
            "https://worker.tail123.ts.net.",
            "https://worker%2etail123.ts.net",
            "https://worker.tail123.ts.net\\evil",
            "https://worker.tail123.ts.net\n",
        ] {
            let mut d = definition();
            d.serve_origin = origin.into();
            assert_eq!(
                d.validate().unwrap_err().path,
                "serve_origin",
                "Accepted {origin}"
            );
        }
        let mut d = definition();
        d.serve_origin = format!(
            "https://{}.{}.{}.{}.tail.ts.net",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(63)
        );
        assert!(d.validate().is_err());
    }
    #[test]
    fn untrusted_report_intersection_is_a_ceiling_not_verified_execution_authority() {
        let d = definition();
        let report = capabilities();
        report.validate().unwrap();
        let effective = d.capability_policy.effective(&report).unwrap();
        assert_eq!(effective.max_parallel, 2);
        assert_eq!(effective.harnesses, vec!["codex"]);
        let mut lower = report;
        lower.max_parallel = 1;
        lower.harnesses.clear();
        assert_eq!(
            d.capability_policy.effective(&lower).unwrap(),
            HostCapabilityPolicy {
                max_parallel: 1,
                harnesses: vec![]
            }
        );
        assert_eq!(
            serde_json::to_value(NetworkVerification::Unknown).unwrap(),
            "unverified"
        );
        assert!(serde_json::from_str::<NetworkVerification>("\"verified\"").is_err());
        assert!(serde_json::from_str::<NetworkVerification>("true").is_err());
        for status in [
            HostStatus::Pending,
            HostStatus::Claimed,
            HostStatus::Approved,
            HostStatus::Revoked,
        ] {
            assert_eq!(
                serde_json::from_value::<HostStatus>(serde_json::to_value(status).unwrap())
                    .unwrap(),
                status
            );
        }
    }
    #[test]
    fn fixed_harnesses_capacity_and_report_bounds_are_enforced() {
        for parallel in [0, 33, u16::MAX] {
            let mut d = definition();
            d.capability_policy.max_parallel = parallel;
            assert!(d.validate().is_err());
            let mut r = capabilities();
            r.max_parallel = parallel;
            assert!(r.validate().is_err());
        }
        for ids in [
            vec!["codex", "codex"],
            vec!["/bin/sh"],
            vec!["codex --unsafe"],
            vec!["unknown"],
            vec!["opencode", "codex", "claude", "opencode"],
        ] {
            let values = ids.into_iter().map(str::to_string).collect::<Vec<_>>();
            let mut d = definition();
            d.capability_policy.harnesses = values.clone();
            assert!(d.validate().is_err());
            let mut r = capabilities();
            r.harnesses = values;
            assert!(r.validate().is_err());
        }
        let mut r = capabilities();
        r.protocol_version = 2;
        assert_eq!(r.validate().unwrap_err().path, "protocol_version");
        let mut r = capabilities();
        r.version = "界".repeat(43);
        assert_eq!(r.validate().unwrap_err().path, "version");
        let mut r = capabilities();
        r.architecture.push('\u{1b}');
        assert_eq!(r.validate().unwrap_err().path, "architecture");
        let mut d = definition();
        d.node_id = "node/key".into();
        assert_eq!(d.validate().unwrap_err().path, "node_id");
        let mut d = definition();
        d.name = "界".repeat(43);
        assert_eq!(d.validate().unwrap_err().path, "name");
    }
    #[test]
    fn wire_roundtrips_and_cannot_set_trusted_flags_or_commands() {
        let d = definition();
        let bytes = serde_json::to_vec(&d).unwrap();
        assert!(bytes.len() < MAX_HOST_DEFINITION_BYTES);
        assert_eq!(serde_json::from_slice::<HostDefinition>(&bytes).unwrap(), d);
        let r = capabilities();
        let bytes = serde_json::to_vec(&r).unwrap();
        assert!(bytes.len() < MAX_HOST_CAPABILITIES_BYTES);
        assert_eq!(
            serde_json::from_slice::<HostCapabilities>(&bytes).unwrap(),
            r
        );
        for field in [
            "execution_ready",
            "network_verification",
            "verified",
            "command",
            "credentials",
        ] {
            let mut value = serde_json::to_value(&d).unwrap();
            value[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<HostDefinition>(value).is_err());
            let mut value = serde_json::to_value(&r).unwrap();
            value[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<HostCapabilities>(value).is_err());
        }
        assert_eq!(serde_json::to_value(SandboxStrength::None).unwrap(), "none");
        assert_eq!(
            serde_json::to_value(SandboxStrength::MacosSandboxExec).unwrap(),
            "macos_sandbox_exec"
        );
    }
}
