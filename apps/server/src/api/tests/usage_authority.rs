use super::*;
use crate::api::AuthUser;
use crate::management::authority::AdminAuthority;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct UsageFixture {
    directory: tempfile::TempDir,
    state: AppState,
    user: String,
    device: String,
    token: String,
}
impl UsageFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("usage-admin", "admin").unwrap();
        let (device, token) = store.create_device(&user.id, "usage-device").unwrap();
        let state = AppState::new(
            store,
            Config {
                db: path,
                sandbox: false,
                ..Default::default()
            },
        );
        Self {
            directory,
            state,
            user: user.id,
            device: device.id,
            token,
        }
    }
    fn auth(&self) -> AuthUser {
        let (user, device) = self
            .state
            .store
            .lock()
            .unwrap()
            .authenticate(&self.token)
            .unwrap()
            .unwrap();
        AuthUser { user, device }
    }
    fn downgrade(&self) {
        rusqlite::Connection::open(self.directory.path().join("relay.db"))
            .unwrap()
            .execute("UPDATE users SET role='member' WHERE id=?1", [&self.user])
            .unwrap();
    }
}
fn private_usage() -> Vec<dunlin_domain::ProviderUsage> {
    vec![dunlin_domain::ProviderUsage {
        provider: "local-fixture".into(),
        plan: "fixture-plan".into(),
        accent: dunlin_domain::ProjectTone::Blue,
        windows: vec![],
        detail: "PRIVATE_USAGE_ACCOUNT_MARKER".into(),
    }]
}

#[tokio::test]
async fn usage_cache_requires_current_admin_and_device_even_with_a_cached_extractor_snapshot() {
    let f = UsageFixture::new();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let first = crate::api::usage_with_probe(f.auth(), f.state.clone(), move |allowed| {
        assert!(allowed());
        calls.fetch_add(1, Ordering::SeqCst);
        private_usage()
    })
    .await
    .unwrap();
    assert_eq!(first.0[0].detail, "PRIVATE_USAGE_ACCOUNT_MARKER");
    let cached = crate::api::usage_with_probe(f.auth(), f.state.clone(), |_| {
        panic!("A fresh authorized cache hit must not invoke a provider")
    })
    .await
    .unwrap();
    assert_eq!(cached.0[0].detail, first.0[0].detail);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let stale = f.auth();
    f.downgrade();
    let error = crate::api::usage_with_probe(stale, f.state.clone(), |_| {
        panic!("A former admin must not probe")
    })
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!error.message.contains("PRIVATE_USAGE_ACCOUNT_MARKER"));
    let other = UsageFixture::new();
    *other.state.usage_cache.lock().unwrap() = Some((std::time::Instant::now(), private_usage()));
    let stale = other.auth();
    other
        .state
        .store
        .lock()
        .unwrap()
        .revoke_device(&other.device)
        .unwrap();
    assert_eq!(
        crate::api::usage_with_probe(stale, other.state.clone(), |_| panic!(
            "A revoked device must not probe"
        ))
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn usage_downgrade_or_device_revocation_during_a_controlled_probe_returns_no_output_or_cache()
{
    for revoke_device in [false, true] {
        let f = UsageFixture::new();
        let permits = f.state.history_reads.available_permits();
        let auth = f.auth();
        let state = f.state.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::spawn(crate::api::usage_with_probe(auth, state, move |allowed| {
            assert!(allowed());
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(
                !allowed(),
                "The same callback handed to real probes must notice lost authority"
            );
            private_usage()
        }));
        started.await.unwrap();
        assert_eq!(f.state.history_reads.available_permits(), permits - 1);
        if revoke_device {
            let store = f.state.store.lock().unwrap();
            store
                .create_device(&f.user, "replacement-is-not-the-request-device")
                .unwrap();
            store.revoke_device(&f.device).unwrap();
        } else {
            f.downgrade();
        }
        release.send(()).unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(
            error.status,
            if revoke_device {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            }
        );
        assert!(!error.message.contains("PRIVATE_USAGE_ACCOUNT_MARKER"));
        assert!(f.state.usage_cache.lock().unwrap().is_none());
        assert_eq!(f.state.history_reads.available_permits(), permits);
    }
}

#[tokio::test]
async fn stale_admin_proof_cannot_publish_cache_or_read_aggregated_billing_rows() {
    let f = UsageFixture::new();
    let proof = AdminAuthority::capture(&f.state, &f.auth()).unwrap();
    f.downgrade();
    assert_eq!(
        crate::api::publish_usage_cache(&f.state, &proof, &private_usage())
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert!(f.state.usage_cache.lock().unwrap().is_none());
    assert_eq!(
        crate::api::aggregate_billing_rows(&f.state, &proof)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn dropping_usage_request_cancels_its_probe_callback_and_retains_permit_until_worker_settles()
{
    let f = UsageFixture::new();
    let permits = f.state.history_reads.available_permits();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (stopped, finished) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(crate::api::usage_with_probe(
        f.auth(),
        f.state.clone(),
        move |allowed| {
            entered.send(()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while allowed() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            stopped.send(()).unwrap();
            private_usage()
        },
    ));
    started.await.unwrap();
    assert_eq!(f.state.history_reads.available_permits(), permits - 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    finished.await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.state.history_reads.available_permits() != permits {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(f.state.usage_cache.lock().unwrap().is_none());
}

fn evidence() -> dunlin_usage::billing::Evidence {
    dunlin_usage::billing::Evidence::new(
        "caller-supplied-display-label",
        dunlin_usage::billing::Summary {
            generated_ms: 1,
            totals: dunlin_usage::billing::Totals {
                events: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn billing_submission_stays_member_owned_device_scoped_and_revoked_proof_cannot_save() {
    let f = Fixture::new();
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let stale_auth = AuthUser {
        user: user.clone(),
        device: device.clone(),
    };
    let proof = crate::api::BillingDeviceAuthority::capture(&f.state, &stale_auth).unwrap();
    let result = crate::api::billing_evidence_put(
        AuthUser {
            user: user.clone(),
            device: device.clone(),
        },
        axum::extract::State(f.state.clone()),
        axum::Json(evidence()),
    )
    .await;
    assert_eq!(
        result.unwrap(),
        StatusCode::OK,
        "A member must retain own-device submission access"
    );
    let initial = {
        let store = f.state.store.lock().unwrap();
        let rows = store.billing_evidence(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, user.id);
        assert_eq!(rows[0].device_id, device.id);
        rows[0].payload.clone()
    };
    {
        let store = f.state.store.lock().unwrap();
        store.create_device(&user.id, "replacement").unwrap();
        store.revoke_device(&device.id).unwrap();
    }
    assert_eq!(
        crate::api::save_billing_payload(&f.state, &proof, "must not replace evidence")
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        crate::api::billing_evidence_put(
            stale_auth,
            axum::extract::State(f.state.clone()),
            axum::Json(evidence())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .billing_evidence(None)
            .unwrap()[0]
            .payload,
        initial
    );
}

#[tokio::test]
async fn aggregated_billing_get_never_trusts_a_cached_admin_claim_or_revoked_admin_device() {
    let f = Fixture::new();
    let (mut user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    user.role = "admin".into();
    let result = crate::api::billing_evidence_get(
        AuthUser { user, device },
        axum::extract::State(f.state.clone()),
    )
    .await;
    assert_eq!(result.err().unwrap().status, StatusCode::FORBIDDEN);
    let admin = UsageFixture::new();
    let auth = admin.auth();
    admin
        .state
        .store
        .lock()
        .unwrap()
        .revoke_device(&admin.device)
        .unwrap();
    assert_eq!(
        crate::api::billing_evidence_get(auth, axum::extract::State(admin.state.clone()))
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::UNAUTHORIZED
    );
}
