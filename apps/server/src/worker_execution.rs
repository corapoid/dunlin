//! Internal v3 fingerprint codec. These helpers only encode and hash typed
//! values; callers must separately validate DTOs, exact published profile bytes,
//! identity/authority epochs, current channel and durable execution state.
pub(crate) mod runtime;
pub(crate) mod session_acp;
use dunlin_domain::agent::AgentDefinition;
use dunlin_relay_protocol::worker::{ExecutionCommand, ExecutionOutcome};
use sha2::{Digest, Sha256};

pub(crate) fn command_fingerprint(command: &ExecutionCommand) -> serde_json::Result<String> {
    let mut value = command.clone();
    value.identity.fingerprint.clear();
    let mut digest = Sha256::new();
    digest.update(b"dunlin.worker.execution.command.v1\0");
    digest.update(serde_json::to_vec(&value)?);
    Ok(hex::encode(digest.finalize()))
}
/// Identical to the existing Store AgentVersion digest: no domain prefix.
pub(crate) fn profile_digest(definition: &AgentDefinition) -> serde_json::Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(definition)?)))
}
pub(crate) fn outcome_digest(outcome: &ExecutionOutcome) -> serde_json::Result<String> {
    let mut value = outcome.clone();
    value.digest.clear();
    let mut digest = Sha256::new();
    digest.update(b"dunlin.worker.execution.outcome.v1\0");
    digest.update(serde_json::to_vec(&value)?);
    Ok(hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_relay_protocol::worker::ExecutionTerminalStatus;
    use serde_json::json;
    fn definition() -> AgentDefinition {
        serde_json::from_value(json!({"schema_version":1,"name":"Published fixture",
            "description":"Literal instructions","harness":"opencode","model":"configured-model",
            "mode":"configured-mode","instructions":"Follow the configured limits.",
            "skills":[{"id":"inspect","version":"1.0","instructions":"Inspect the requested scope."}],
            "max_seconds":120,"max_output_bytes":65536})).unwrap()
    }
    fn command() -> ExecutionCommand {
        serde_json::from_value(json!({
            "channel":{"channel_id":"channel-1","controller_boot_id":"controller-boot","worker_boot_id":"worker-boot"},
            "identity":{"command_id":"command-1","attempt_id":"attempt-1","fingerprint":"a".repeat(64)},
            "descriptor":{
                "scope":{"controller_node_id":"controller-node","worker_node_id":"worker-node",
                    "workspace_id":"workspace-1","host_id":"host-1","enrollment_epoch":1,"credential_revision":1},
                "export":{"id":"export-1","name":"Export","repo_kind":"git","checkout_identity_digest":"b".repeat(64),
                    "settings_digest":"c".repeat(64),"revision":1,"active":true},
                "grant":{"principal":{"kind":"human","id":"user-1"},"permissions":["read","execute"],
                    "revision":1,"export_revision":1,"active":true}},
            "profile":{"agent_id":"agent-1","version":1,"digest":profile_digest(&definition()).unwrap()},
            "definition":definition(),"input":"Inspect the explicit request.",
            "budgets":{"max_seconds":60,"max_output_bytes":32768,"lease_duration_ms":10000}
        })).unwrap()
    }
    #[test]
    fn command_integrity_includes_every_scope_pin_boot_input_and_budget_except_its_self_digest() {
        let original = command();
        original.validate().unwrap();
        let expected = command_fingerprint(&original).unwrap();
        let mut self_changed = original.clone();
        self_changed.identity.fingerprint = expected.clone();
        assert_eq!(command_fingerprint(&self_changed).unwrap(), expected);
        for (pointer, changed) in [
            ("/channel/channel_id", json!("replacement-channel")),
            ("/channel/controller_boot_id", json!("new-controller-boot")),
            ("/channel/worker_boot_id", json!("new-worker-boot")),
            ("/identity/command_id", json!("another-command")),
            ("/identity/attempt_id", json!("another-attempt")),
            (
                "/descriptor/scope/controller_node_id",
                json!("other-controller"),
            ),
            ("/descriptor/scope/worker_node_id", json!("other-worker")),
            ("/descriptor/scope/workspace_id", json!("another-workspace")),
            ("/descriptor/scope/host_id", json!("another-host")),
            ("/descriptor/scope/enrollment_epoch", json!(2)),
            ("/descriptor/scope/credential_revision", json!(2)),
            ("/descriptor/export/id", json!("other-export")),
            ("/descriptor/export/revision", json!(2)),
            (
                "/descriptor/export/checkout_identity_digest",
                json!("d".repeat(64)),
            ),
            ("/descriptor/export/settings_digest", json!("e".repeat(64))),
            ("/descriptor/grant/principal/id", json!("another-user")),
            ("/descriptor/grant/revision", json!(2)),
            ("/descriptor/grant/permissions", json!(["read"])),
            ("/profile/agent_id", json!("another-agent")),
            ("/profile/version", json!(2)),
            ("/profile/digest", json!("f".repeat(64))),
            ("/definition/model", json!("different-model")),
            ("/definition/instructions", json!("different instructions")),
            ("/input", json!("Different explicit request.")),
            ("/budgets/max_seconds", json!(30)),
            ("/budgets/max_output_bytes", json!(16384)),
            ("/budgets/lease_duration_ms", json!(5000)),
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            *value.pointer_mut(pointer).unwrap() = changed;
            let changed = serde_json::from_value(value).unwrap();
            assert_ne!(
                command_fingerprint(&changed).unwrap(),
                expected,
                "Unbound field {pointer}"
            );
        }
    }
    #[test]
    fn profile_codec_matches_a_real_published_store_version_and_covers_skill_instructions() {
        let store = crate::store::Store::open_in_memory().unwrap();
        let owner = store.create_user("worker-profile-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "Profiles").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "Worker profile")
            .unwrap();
        let draft = store
            .create_agent(&owner.id, &workspace.id, &project.id, definition())
            .unwrap();
        let version = store
            .publish_agent(
                &owner.id,
                &workspace.id,
                &project.id,
                &draft.id,
                draft.revision,
            )
            .unwrap();
        assert_eq!(profile_digest(&version.definition).unwrap(), version.digest);
        let mut changed = version.definition;
        changed.skills[0]
            .instructions
            .push_str(" Extra explicit rule.");
        assert_ne!(profile_digest(&changed).unwrap(), version.digest);
    }
    #[test]
    fn outcome_codec_has_a_fixed_domain_separated_golden_and_excludes_only_its_digest() {
        let mut value = ExecutionOutcome {
            outcome_id: "outcome-1".into(),
            digest: String::new(),
            status: ExecutionTerminalStatus::Completed,
            text: "done".into(),
        };
        let expected = outcome_digest(&value).unwrap();
        assert_eq!(
            expected,
            "358f44ca94b8cf289627f5e895155e2f724b85031c71a3099d778980dd82a219"
        );
        value.digest = expected.clone();
        assert_eq!(outcome_digest(&value).unwrap(), expected);
        let mut changed = value.clone();
        changed.outcome_id = "outcome-2".into();
        assert_ne!(outcome_digest(&changed).unwrap(), expected);
        let mut changed = value.clone();
        changed.status = ExecutionTerminalStatus::Unknown;
        assert_ne!(outcome_digest(&changed).unwrap(), expected);
        value.text.push('!');
        assert_ne!(outcome_digest(&value).unwrap(), expected);
    }
}
