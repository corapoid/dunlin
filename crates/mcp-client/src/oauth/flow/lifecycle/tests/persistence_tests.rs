use super::*;
use crate::oauth::persistence::{
    CredentialIntent, CredentialJournal, CredentialRecord, PersistenceError,
};
use sha2::Digest;
use std::cell::{Cell, RefCell};
use zeroize::Zeroizing;

// Fault-injection journal. SQLite durability/concurrency is separately exercised
// against the concrete server implementation; these tests inspect HTTP ordering.
#[derive(Default)]
struct Journal {
    record: RefCell<Option<CredentialRecord>>,
    pending: RefCell<Option<CredentialIntent>>,
    fail_reserve: Cell<bool>,
    fail_commit: Cell<bool>,
    prepared: RefCell<Option<(CredentialIntent, String)>>,
    fail_prepare: Cell<bool>,
    claimed: RefCell<Option<(CredentialIntent, String)>>,
    acknowledged: RefCell<Option<(CredentialIntent, String)>>,
    fail_claim: Cell<bool>,
    fail_ack: Cell<bool>,
    disabled: Cell<bool>,
}
impl CredentialJournal for Journal {
    fn active(&self, _: &AuthContext, _: &str) -> Result<CredentialRecord, PersistenceError> {
        if self.disabled.get() || self.pending.borrow().is_some() {
            return Err(PersistenceError::Conflict);
        }
        self.record
            .borrow()
            .clone()
            .ok_or(PersistenceError::Conflict)
    }
    fn begin_install(
        &self,
        _: &AuthContext,
        _: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        if self.record.borrow().is_some() || self.pending.borrow().is_some() {
            return Err(PersistenceError::Conflict);
        }
        let intent = CredentialIntent {
            generation: 1,
            operation_id: "install".into(),
            staged_reference: "private-one".into(),
        };
        *self.pending.borrow_mut() = Some(intent.clone());
        Ok(intent)
    }
    fn begin_refresh(
        &self,
        context: &AuthContext,
        binding: &str,
        record: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        if self.fail_reserve.get() || self.active(context, binding)? != *record {
            return Err(PersistenceError::Conflict);
        }
        let intent = CredentialIntent {
            generation: record.generation + 1,
            operation_id: "refresh".into(),
            staged_reference: "private-two".into(),
        };
        *self.pending.borrow_mut() = Some(intent.clone());
        Ok(intent)
    }
    fn prepare_write(
        &self,
        _: &AuthContext,
        _: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError> {
        if self.fail_prepare.get()
            || self.disabled.get()
            || self.pending.borrow().as_ref() != Some(intent)
        {
            return Err(PersistenceError::Conflict);
        }
        let value = (intent.clone(), sha256.to_owned());
        if self
            .prepared
            .borrow()
            .as_ref()
            .is_some_and(|old| old.0 == *intent && old != &value)
        {
            return Err(PersistenceError::Conflict);
        }
        *self.prepared.borrow_mut() = Some(value);
        Ok(())
    }
    fn claim_write(
        &self,
        _: &AuthContext,
        _: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        let value = (i.clone(), h.to_owned());
        if self.disabled.get()
            || self.fail_claim.get()
            || self.prepared.borrow().as_ref() != Some(&value)
            || self.pending.borrow().as_ref() != Some(i)
            || self.claimed.borrow().as_ref() == Some(&value)
        {
            return Err(PersistenceError::Conflict);
        }
        *self.claimed.borrow_mut() = Some(value);
        Ok(())
    }
    fn acknowledge_write(
        &self,
        _: &AuthContext,
        _: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        let value = (i.clone(), h.to_owned());
        if self.fail_ack.get()
            || self.claimed.borrow().as_ref() != Some(&value)
            || self.acknowledged.borrow().as_ref() == Some(&value)
        {
            return Err(PersistenceError::Storage);
        }
        *self.acknowledged.borrow_mut() = Some(value);
        Ok(())
    }
    fn commit(
        &self,
        _: &AuthContext,
        _: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError> {
        if self.prepared.borrow().as_ref() != Some(&(intent.clone(), sha256.to_owned()))
            || self.acknowledged.borrow().as_ref() != Some(&(intent.clone(), sha256.to_owned()))
            || self.fail_commit.get()
            || self.disabled.get()
            || self.pending.borrow().as_ref() != Some(intent)
        {
            return Err(PersistenceError::Conflict);
        }
        *self.record.borrow_mut() = Some(CredentialRecord {
            generation: intent.generation,
            secret_reference: intent.staged_reference.clone(),
            sha256: sha256.into(),
        });
        self.pending.borrow_mut().take();
        Ok(())
    }
    fn disable(&self, _: &AuthContext) -> Result<u64, PersistenceError> {
        self.disabled.set(true);
        Ok(2)
    }
}
fn vault_get(
    vault: &RefCell<BTreeMap<String, Zeroizing<String>>>,
    id: &str,
) -> Result<Zeroizing<String>, PersistenceError> {
    vault
        .borrow()
        .get(id)
        .cloned()
        .ok_or(PersistenceError::Storage)
}
fn vault_put(
    vault: &RefCell<BTreeMap<String, Zeroizing<String>>>,
    id: &str,
    value: &str,
) -> Result<(), PersistenceError> {
    assert!(
        !vault.borrow().contains_key(id),
        "never overwrite a credential generation"
    );
    vault
        .borrow_mut()
        .insert(id.into(), Zeroizing::new(value.into()));
    Ok(())
}

#[test]
fn dispatch_preparation_keeps_valid_generation_without_refresh() {
    let (server, flow, token) = open(vec![], "public", false);
    let journal = Journal::default();
    let vault = RefCell::new(BTreeMap::new());
    let installed = flow
        .install_durable(&context(), token, &journal, |id, v| {
            vault_put(&vault, id, v)
        })
        .unwrap();
    let (record, _) = flow
        .access_token_or_refresh_durable(
            &context(),
            &journal,
            |id| vault_get(&vault, id),
            |_, _| panic!("valid credentials do not rotate"),
        )
        .unwrap();
    assert_eq!(record, installed);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn expired_dispatch_preparation_cannot_rotate_a_concurrently_replaced_generation() {
    let (server, flow, mut token) = open(vec![], "public", false);
    token.expires = Instant::now() - Duration::from_secs(1);
    let journal = Journal::default();
    let vault = RefCell::new(BTreeMap::new());
    flow.install_durable(&context(), token, &journal, |id, v| {
        vault_put(&vault, id, v)
    })
    .unwrap();
    assert!(
        flow.access_token_or_refresh_durable(
            &context(),
            &journal,
            |id| {
                let value = vault_get(&vault, id)?;
                journal.record.borrow_mut().as_mut().unwrap().generation += 1;
                Ok(value)
            },
            |_, _| panic!("the expired generation lost CAS before rotation")
        )
        .is_err()
    );
    assert!(journal.pending.borrow().is_none());
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn dispatch_preparation_refreshes_local_expiry_once_and_never_retries_uncertain_rotation() {
    for lost in [false, true] {
        let (server, flow, mut token) = open(
            vec![(
                "/token",
                if lost { 0 } else { 200 },
                refreshed("access-two", Some("refresh-two")),
            )],
            "public",
            false,
        );
        token.expires = Instant::now() - Duration::from_secs(1);
        let journal = Journal::default();
        let vault = RefCell::new(BTreeMap::new());
        flow.install_durable(&context(), token, &journal, |id, v| {
            vault_put(&vault, id, v)
        })
        .unwrap();
        let result = flow.access_token_or_refresh_durable(
            &context(),
            &journal,
            |id| vault_get(&vault, id),
            |id, v| vault_put(&vault, id, v),
        );
        if lost {
            assert!(result.is_err());
            assert!(
                flow.access_token_or_refresh_durable(
                    &context(),
                    &journal,
                    |_| panic!("uncertain rotation is blocked before vault"),
                    |_, _| panic!("no retry")
                )
                .is_err()
            );
        } else {
            let (record, bearer) = result.unwrap();
            assert_eq!(record.generation, 2);
            assert_eq!(&*bearer, "access-two");
        }
        assert_eq!(server.finish().len(), 2);
    }
}
#[test]
fn durable_refresh_publishes_only_after_secret_write_and_survives_flow_reconstruction() {
    let (server, flow, token) = open(
        vec![("/token", 200, refreshed("access-two", Some("refresh-two")))],
        "public",
        false,
    );
    let journal = Journal::default();
    let vault = RefCell::new(BTreeMap::new());
    let first = flow
        .install_durable(&context(), token, &journal, |id, value| {
            vault_put(&vault, id, value)
        })
        .unwrap();
    let access_before = flow
        .access_token_durable(&context(), &journal, |id| vault_get(&vault, id))
        .unwrap();
    let second = flow
        .refresh_durable(
            &context(),
            &journal,
            |id| vault_get(&vault, id),
            |id, value| {
                assert!(
                    journal
                        .active(&context(), &flow.credential_binding())
                        .is_err()
                );
                vault_put(&vault, id, value)
            },
        )
        .unwrap();
    assert_ne!(first.secret_reference, second.secret_reference);
    assert_eq!(second.generation, 2);
    let access = flow
        .access_token_durable(&context(), &journal, |id| vault_get(&vault, id))
        .unwrap();
    assert_ne!(*access_before, *access);
    assert_eq!(&*access, "access-two");
    // Reconstruct flow with no in-memory token or authorization state.
    let discovery = Discovery::new(&flow.endpoint, &[]).unwrap();
    let resource = ResourceMetadata {
        resource: flow.resource.clone(),
        authorization_servers: vec![flow.issuer.clone()],
        scopes_supported: None,
    };
    let reconstructed = discovery
        .authorization_flow(
            &resource,
            metadata(&server.origin, true),
            registration(&server.origin, "public"),
        )
        .unwrap();
    drop(flow);
    assert_eq!(
        &*reconstructed
            .access_token_durable(&context(), &journal, |id| vault_get(&vault, id))
            .unwrap(),
        "access-two"
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].form["grant_type"], "refresh_token");
}
#[test]
fn durable_refresh_reservation_failure_or_lost_authorization_sends_no_http() {
    for deactivate_during_read in [false, true] {
        let (server, flow, token) = open(vec![], "public", false);
        let journal = Journal::default();
        let vault = RefCell::new(BTreeMap::new());
        flow.install_durable(&context(), token, &journal, |id, v| {
            vault_put(&vault, id, v)
        })
        .unwrap();
        journal.fail_reserve.set(!deactivate_during_read);
        assert!(
            flow.refresh_durable(
                &context(),
                &journal,
                |id| {
                    let result = vault_get(&vault, id);
                    if deactivate_during_read {
                        journal.disable(&context()).unwrap();
                    }
                    result
                },
                |_, _| panic!("no secret write before reservation")
            )
            .is_err()
        );
        assert_eq!(server.finish().len(), 1);
    }
}
#[test]
fn durable_refresh_lost_response_or_vault_or_commit_failure_never_retries_old_token() {
    for fault in ["response", "vault", "commit"] {
        let status = if fault == "response" { 0 } else { 200 };
        let (server, flow, token) = open(
            vec![(
                "/token",
                status,
                refreshed("access-two", Some("refresh-two")),
            )],
            "public",
            false,
        );
        let journal = Journal::default();
        let vault = RefCell::new(BTreeMap::new());
        flow.install_durable(&context(), token, &journal, |id, v| {
            vault_put(&vault, id, v)
        })
        .unwrap();
        journal.fail_commit.set(fault == "commit");
        assert!(
            flow.refresh_durable(
                &context(),
                &journal,
                |id| vault_get(&vault, id),
                |id, v| {
                    if fault == "vault" {
                        return Err(PersistenceError::Storage);
                    }
                    vault_put(&vault, id, v)
                }
            )
            .is_err()
        );
        assert!(
            flow.access_token_durable(&context(), &journal, |_| panic!(
                "blocked before vault read"
            ))
            .is_err()
        );
        assert!(
            flow.refresh_durable(
                &context(),
                &journal,
                |_| panic!("blocked before vault read"),
                |_, _| panic!("blocked before vault write")
            )
            .is_err()
        );
        assert_eq!(server.finish().len(), 2);
    }
}
#[test]
fn durable_restore_rejects_swapped_corrupt_expired_or_clock_rollback_credentials() {
    for fault in ["digest", "generation", "user", "expired", "clock"] {
        let (server, flow, token) = open(vec![], "public", false);
        let journal = Journal::default();
        let vault = RefCell::new(BTreeMap::new());
        flow.install_durable(&context(), token, &journal, |id, v| {
            vault_put(&vault, id, v)
        })
        .unwrap();
        let mut record = journal.record.borrow().clone().unwrap();
        let original = vault_get(&vault, &record.secret_reference).unwrap();
        let mut value: Value = serde_json::from_str(&original).unwrap();
        match fault {
            "digest" => value["access_token"] = json!("wrong-access"),
            "generation" => value["generation"] = json!(2),
            "user" => value["user_id"] = json!("another-user"),
            "expired" => value["expires_at_ms"] = value["saved_at_ms"].clone(),
            _ => {
                value["saved_at_ms"] = json!(u64::MAX - 1);
                value["expires_at_ms"] = json!(u64::MAX);
            }
        }
        let value = Zeroizing::new(value.to_string());
        if fault != "digest" {
            use sha2::{Digest, Sha256};
            record.sha256 = hex::encode(Sha256::digest(value.as_bytes()));
            *journal.record.borrow_mut() = Some(record.clone());
        }
        vault.borrow_mut().insert(record.secret_reference, value);
        assert!(
            flow.access_token_durable(&context(), &journal, |id| vault_get(&vault, id))
                .is_err()
        );
        assert_eq!(server.finish().len(), 1);
    }
}
#[test]
fn durable_install_failure_leaves_no_active_secret_and_dispatch_rechecks_authorization() {
    let (server, flow, token) = open(vec![], "public", false);
    let journal = Journal::default();
    assert!(
        flow.install_durable(&context(), token, &journal, |_, _| Err(
            PersistenceError::Storage
        ))
        .is_err()
    );
    assert!(
        flow.access_token_durable(&context(), &journal, |_| panic!("no active record"))
            .is_err()
    );
    assert_eq!(server.finish().len(), 1);
    let (server, flow, token) = open(vec![], "public", false);
    let journal = Journal::default();
    let vault = RefCell::new(BTreeMap::new());
    flow.install_durable(&context(), token, &journal, |id, v| {
        vault_put(&vault, id, v)
    })
    .unwrap();
    assert!(
        flow.access_token_durable(&context(), &journal, |id| {
            let value = vault_get(&vault, id);
            journal.disable(&context()).unwrap();
            value
        })
        .is_err()
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn durable_publish_records_identity_before_secret_io_and_never_writes_after_prepare_failure() {
    for fail_prepare in [false, true] {
        let (server, flow, token) = open(vec![], "public", false);
        let journal = Journal::default();
        journal.fail_prepare.set(fail_prepare);
        let writes = Cell::new(0);
        let result = flow.install_durable(&context(), token, &journal, |id, value| {
            writes.set(writes.get() + 1);
            let (intent, hash) = journal.prepared.borrow().clone().unwrap();
            assert_eq!(id, intent.staged_reference);
            assert_eq!(hash, hex::encode(sha2::Sha256::digest(value.as_bytes())));
            // Even an uncertain write retains its exact envelope identity.
            Err(PersistenceError::Storage)
        });
        assert!(result.is_err());
        assert_eq!(writes.get(), usize::from(!fail_prepare));
        assert_eq!(journal.prepared.borrow().is_some(), !fail_prepare);
        assert!(
            journal
                .active(&context(), &flow.credential_binding())
                .is_err()
        );
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn durable_claim_is_one_use_and_failure_prevents_private_io() {
    let (server, flow, token) = open(vec![], "public", false);
    let journal = Journal::default();
    journal.fail_claim.set(true);
    let writes = Cell::new(0);
    assert!(
        flow.install_durable(&context(), token, &journal, |_, _| {
            writes.set(writes.get() + 1);
            Ok(())
        })
        .is_err()
    );
    assert_eq!(writes.get(), 0);
    assert!(journal.acknowledged.borrow().is_none());
    assert!(journal.record.borrow().is_none());
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn durable_backend_ack_is_after_success_and_survives_fencing_without_activating() {
    for fault in ["put", "ack", "commit", "revoked", "none"] {
        let (server, flow, token) = open(vec![], "public", false);
        let journal = Journal::default();
        let writes = Cell::new(0);
        journal.fail_ack.set(fault == "ack");
        journal.fail_commit.set(fault == "commit");
        let result = flow.install_durable(&context(), token, &journal, |_, _| {
            writes.set(writes.get() + 1);
            assert!(journal.claimed.borrow().is_some());
            assert!(journal.acknowledged.borrow().is_none());
            if fault == "revoked" {
                journal.disable(&context()).unwrap();
            }
            if fault == "put" {
                Err(PersistenceError::Storage)
            } else {
                Ok(())
            }
        });
        assert_eq!(writes.get(), 1);
        assert_eq!(result.is_ok(), fault == "none");
        assert_eq!(
            journal.acknowledged.borrow().is_some(),
            !["put", "ack"].contains(&fault)
        );
        assert_eq!(journal.record.borrow().is_some(), fault == "none");
        let (intent, hash) = journal.claimed.borrow().clone().unwrap();
        assert!(
            journal
                .claim_write(&context(), &flow.credential_binding(), &intent, &hash)
                .is_err()
        );
        assert_eq!(server.finish().len(), 1);
    }
}
