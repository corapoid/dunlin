use super::*;
use dunlin_domain::collaboration::{Permission, WorkspaceRole};
use dunlin_session_runtime::TurnPhase;

struct SharingFixture {
    state: AppState,
    owner_cookie: String,
    reader_cookie: String,
    reader: String,
    session: String,
    epoch: u64,
    _directory: tempfile::TempDir,
}
const PRIVATE_MARKER: &str = "SHARING_HTTP_PRIVATE_PROMPT_AND_OUTPUT";

async fn setup() -> SharingFixture {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("relay.db")).unwrap();
    let owner = store.create_user("sharing-http-owner", "admin").unwrap();
    let reader = store.create_user("sharing-http-reader", "member").unwrap();
    let workspace = store.create_workspace(&owner, "Sharing HTTP").unwrap();
    let project = store
        .create_team_project(&owner.id, &workspace.id, "Project")
        .unwrap();
    store
        .set_team_member(
            &owner.id,
            &workspace.id,
            &reader.id,
            WorkspaceRole::Member,
            true,
            None,
        )
        .unwrap();
    store
        .set_team_project_grant(
            &owner.id,
            &workspace.id,
            &project.id,
            &reader.id,
            [
                Permission::ViewProject,
                Permission::ViewSharedSession,
                Permission::CreateSession,
            ]
            .into_iter()
            .collect(),
            None,
        )
        .unwrap();
    let authority = store
        .team_project_context(&workspace.id, &project.id, &owner.id)
        .unwrap();
    let journal = store.interactive_journal();
    let session = journal
        .create(&authority, &workspace.id, &project.id)
        .unwrap();
    let lease = journal
        .acquire(&authority, &session.id, 0, "sharing-http-fixture")
        .unwrap();
    let turn = journal
        .enqueue(&authority, &session.id, "http-turn", PRIVATE_MARKER)
        .unwrap();
    journal.claim_next(&authority, &session.id, &lease).unwrap();
    journal
        .transition(&authority, &session.id, &lease, &turn, TurnPhase::Running)
        .unwrap();
    journal
        .output(&authority, &session.id, &lease, &turn, PRIVATE_MARKER)
        .unwrap();
    drop(journal);
    let owner_code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let reader_code = store.create_pairing_code(&reader.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            db: directory.path().join("relay.db"),
            sandbox: false,
            experimental_interactive_runtime: true,
            store_transcripts: true,
            ..Default::default()
        },
    );
    let cookie = |response: &Response| {
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    };
    let owner_response = browser_pair(&state, &owner_code).await;
    let reader_response = browser_pair(&state, &reader_code).await;
    assert_eq!(owner_response.status(), StatusCode::OK);
    assert_eq!(reader_response.status(), StatusCode::OK);
    SharingFixture {
        state,
        owner_cookie: cookie(&owner_response),
        reader_cookie: cookie(&reader_response),
        reader: reader.id,
        session: session.id,
        epoch: lease.epoch,
        _directory: directory,
    }
}

async fn request(
    f: &SharingFixture,
    cookie: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    origin: &str,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "build.tail123.ts.net")
        .header("origin", origin)
        .header("cookie", cookie);
    let body = if let Some(value) = body {
        request = request.header("content-type", "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    super::super::super::router(f.state.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}
async fn same_origin(
    f: &SharingFixture,
    cookie: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Response {
    request(
        f,
        cookie,
        method,
        path,
        body,
        "https://build.tail123.ts.net",
    )
    .await
}
async fn json_response(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    serde_json::from_slice(&to_bytes(response.into_body(), 256 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn sharing_http_grant_is_read_only_and_cas_revoke_preserves_browser_identity() {
    let f = setup().await;
    let sharing = format!("/v1/web/interactive-sessions/{}/sharing", f.session);
    let metadata = format!("/v1/web/interactive-sessions/{}", f.session);
    let stop = format!("/v1/web/interactive-sessions/{}/stop", f.session);
    let initial =
        json_response(same_origin(&f, &f.owner_cookie, "GET", &sharing, None).await).await;
    let first_revision = initial["revision"].as_u64().unwrap();
    assert_eq!(initial["readers"], json!([]));
    assert_eq!(initial["audit"], json!([]));
    assert!(
        initial["eligible_readers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reader| reader["user_id"] == f.reader)
    );
    for path in [&sharing, &metadata] {
        assert_eq!(
            same_origin(&f, &f.reader_cookie, "GET", path, None)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        same_origin(
            &f,
            &f.reader_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":first_revision,"reader_ids":[f.reader]}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );

    let granted = json_response(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":first_revision,"reader_ids":[f.reader]})),
        )
        .await,
    )
    .await;
    let grant_revision = granted["revision"].as_u64().unwrap();
    assert!(grant_revision > first_revision);
    assert_eq!(granted["readers"][0]["user_id"], f.reader);
    assert_eq!(granted["readers"][0]["active"], true);
    let visible =
        json_response(same_origin(&f, &f.reader_cookie, "GET", &metadata, None).await).await;
    assert_eq!(visible["session"]["id"], f.session);
    assert!(
        !visible.to_string().contains(PRIVATE_MARKER),
        "Diagnostics expose bounded metadata, not transcripts"
    );
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", &sharing, None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        same_origin(
            &f,
            &f.reader_cookie,
            "POST",
            &stop,
            Some(json!({"expected_epoch":f.epoch}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "CreateSession on a project must not authorize stopping another owner's session"
    );
    assert_eq!(
        same_origin(
            &f,
            &f.reader_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":grant_revision,"reader_ids":[]}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );

    assert_eq!(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":first_revision,"reader_ids":[]}))
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", &metadata, None)
            .await
            .status(),
        StatusCode::OK
    );
    let revoked = json_response(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":grant_revision,"reader_ids":[]})),
        )
        .await,
    )
    .await;
    assert!(revoked["revision"].as_u64().unwrap() > grant_revision);
    assert_eq!(revoked["readers"], json!([]));
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", &metadata, None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let identity =
        json_response(same_origin(&f, &f.reader_cookie, "GET", "/v1/web/session", None).await)
            .await;
    assert_eq!(identity["user"]["id"], f.reader);
    // A stale owner tab cannot restore an already revoked recipient.
    assert_eq!(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &sharing,
            Some(json!({"expected_revision":grant_revision,"reader_ids":[f.reader]}))
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", &metadata, None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let audit = json_response(same_origin(&f, &f.owner_cookie, "GET", &sharing, None).await).await;
    assert_eq!(audit["revision"], revoked["revision"]);
    let entries = audit["audit"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .any(|entry| entry["added"] == json!([f.reader]))
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry["removed"] == json!([f.reader]))
    );
    assert!(!audit.to_string().contains(PRIVATE_MARKER));
}

#[tokio::test]
async fn sharing_http_cross_origin_read_and_write_cannot_observe_or_change_acl() {
    let f = setup().await;
    let path = format!("/v1/web/interactive-sessions/{}/sharing", f.session);
    let initial = json_response(same_origin(&f, &f.owner_cookie, "GET", &path, None).await).await;
    for (method, body) in [
        ("GET", None),
        (
            "PUT",
            Some(json!({"expected_revision":initial["revision"],"reader_ids":[f.reader]})),
        ),
    ] {
        assert_eq!(
            request(
                &f,
                &f.owner_cookie,
                method,
                &path,
                body,
                "https://evil.example"
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    let current = json_response(same_origin(&f, &f.owner_cookie, "GET", &path, None).await).await;
    assert_eq!(
        current, initial,
        "Rejected cross-origin requests must leave revision, readers and audit unchanged"
    );
}

#[tokio::test]
async fn sharing_http_truncated_recipient_catalog_never_blocks_existing_reader_revocation() {
    let f = setup().await;
    let path = format!("/v1/web/interactive-sessions/{}/sharing", f.session);
    let initial = json_response(same_origin(&f, &f.owner_cookie, "GET", &path, None).await).await;
    let granted = json_response(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &path,
            Some(json!({"expected_revision":initial["revision"],"reader_ids":[f.reader]})),
        )
        .await,
    )
    .await;
    {
        let store = f.state.store.lock().unwrap();
        let (workspace, project, _) = store.interactive_identity(&f.session).unwrap();
        let mut users = Vec::new();
        for index in 0..256 {
            users.push(
                store
                    .create_user(&format!("catalog-boundary-{index:03}"), "member")
                    .unwrap()
                    .id,
            );
        }
        // Model an imported/legacy oversized workspace. The current public
        // membership API caps new memberships at 256, but existing database
        // state must never make an owner's security revocation unavailable.
        let mut connection =
            rusqlite::Connection::open(f._directory.path().join("relay.db")).unwrap();
        let transaction = connection.transaction().unwrap();
        let permissions =
            serde_json::to_string(&[Permission::ViewProject, Permission::ViewSharedSession])
                .unwrap();
        for user in users {
            transaction.execute("INSERT INTO team_memberships(workspace_id,user_id,role,active,revision) VALUES(?1,?2,'member',1,1)", rusqlite::params![workspace,user]).unwrap();
            transaction.execute("INSERT INTO team_project_grants(workspace_id,project_id,user_id,permissions,revision) VALUES(?1,?2,?3,?4,1)", rusqlite::params![workspace,project,user,permissions]).unwrap();
        }
        transaction
            .execute(
                "UPDATE users SET name=?1 WHERE id=?2",
                rusqlite::params![format!("sharing-http-reader{}", "z".repeat(4096)), f.reader],
            )
            .unwrap();
        transaction
            .execute(
                "UPDATE users SET name=name || ?1 WHERE name='catalog-boundary-000'",
                ["x".repeat(4096)],
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    let large = json_response(same_origin(&f, &f.owner_cookie, "GET", &path, None).await).await;
    assert_eq!(large["eligible_readers_truncated"], true);
    assert_eq!(large["eligible_readers"].as_array().unwrap().len(), 256);
    assert!(
        !large["eligible_readers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reader| reader["user_id"] == f.reader),
        "The existing reader is deliberately outside the sorted candidate page"
    );
    assert_eq!(large["readers"][0]["user_id"], f.reader);
    assert_eq!(
        large["readers"][0]["name"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        128
    );
    assert!(
        large["eligible_readers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|reader| reader["name"].as_str().unwrap().encode_utf16().count() <= 256)
    );
    assert_eq!(large["revision"], granted["revision"]);
    let revoked = json_response(
        same_origin(
            &f,
            &f.owner_cookie,
            "PUT",
            &path,
            Some(json!({"expected_revision":large["revision"],"reader_ids":[]})),
        )
        .await,
    )
    .await;
    assert_eq!(revoked["eligible_readers_truncated"], true);
    assert_eq!(revoked["readers"], json!([]));
    let metadata = format!("/v1/web/interactive-sessions/{}", f.session);
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", &metadata, None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        same_origin(&f, &f.reader_cookie, "GET", "/v1/web/session", None)
            .await
            .status(),
        StatusCode::OK
    );
}
