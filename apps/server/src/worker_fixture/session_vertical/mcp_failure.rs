//! Joined failures after a human approval and a real provider HTTP mutation.
use super::*;
use std::path::Path;

pub(super) struct Case<'a> {
    pub fixture: &'a Fixture,
    pub mode: &'a str,
    pub id: &'a str,
    pub path: &'a str,
    pub payload: &'a Value,
    pub cookie: &'a str,
    pub root: &'a Path,
    pub state_dir: &'a Path,
    pub provider: &'a crate::worker_fixture::mcp_provider::Provider,
    pub selection: &'a crate::interactive::mcp::Selection,
    pub faults: &'a crate::worker_client::session_faults::Guard,
}

impl Case<'_> {
    pub async fn verify(&self, socket: &mut SessionSocket, confirmation: Value) {
        if self.mode == "oauth-revoke" {
            self.provider.wait_refresh(false).await;
        } else {
            self.provider.wait_call().await;
        }
        if matches!(
            self.mode,
            "revoke" | "forms-revoke" | "urls-revoke" | "oauth-revoke"
        ) {
            let (status, result) = fixture_http(
                &self.fixture.state,
                self.cookie,
                &format!(
                    "/v1/web/workspaces/{}/mcp-connections/{}/disconnect",
                    self.fixture.workspace, self.selection.connection_id
                ),
                json!({"expected_revision":self.selection.revision}),
            )
            .await;
            assert_eq!(status, 200, "{result}");
            if self.mode == "oauth-revoke" {
                self.provider.wait_refresh(true).await;
                self.provider.assert_refresh(true);
            } else {
                self.provider.release();
            }
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let done = {
                    let store = self.fixture.state.store.lock().unwrap();
                    let snapshot = store.remote_session_snapshot(self.id).unwrap();
                    snapshot.phase == "stopped"
                        && snapshot.sequence == 2
                        && snapshot.ack_sequence == 2
                        && snapshot.confirmed_ack == 2
                };
                if done {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            let store = self.fixture.state.store.lock().unwrap();
            let s = store.remote_session_snapshot(self.id).unwrap();
            panic!(
                "{} stop/ACK deadline: {:?}",
                self.mode,
                (s.phase, s.sequence, s.ack_sequence, s.confirmed_ack)
            );
        });
        {
            let store = self.fixture.state.store.lock().unwrap();
            let snapshot = store.remote_session_snapshot(self.id).unwrap();
            let context = store
                .team_project_context(
                    &self.fixture.workspace,
                    &snapshot.project,
                    &self.fixture.owner,
                )
                .unwrap();
            let (_, turns) = store
                .interactive_journal()
                .snapshot(&context, self.id)
                .unwrap();
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].phase, dunlin_session_runtime::TurnPhase::Unknown);
            let chat = serde_json::to_string(
                &store
                    .interactive_journal()
                    .projected_chat(&context, self.id)
                    .unwrap(),
            )
            .unwrap();
            assert!(!chat.contains("native broker result"));
            assert!(!chat.contains("remembered:journal-context"));
            assert!(!chat.contains("private-mrtr-state"));
            assert!(!chat.contains("private-url-state"));
            if matches!(self.mode, "forms-revoke" | "urls-revoke") {
                let chat: Value = serde_json::from_str(&chat).unwrap();
                assert!(
                    chat["turns"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|t| t["responseParts"].as_array().unwrap())
                        .any(|p| p["kind"] == "inputRequest" && p["response"] == "cancel")
                );
            }
        }
        // Re-delivery of the original human action is historical evidence only.
        session_dispatch(socket, "remote-browser-1", confirmation).await;
        let (status, replay) = session_http(
            &self.fixture.state,
            self.cookie,
            self.path,
            self.payload.clone(),
        )
        .await;
        if matches!(
            self.mode,
            "revoke" | "forms-revoke" | "urls-revoke" | "oauth-revoke"
        ) {
            assert_eq!(status, 409, "{replay}");
        } else {
            assert_eq!(status, 202, "{replay}");
            assert_eq!(replay["admission"]["replayed"], true);
            assert_eq!(replay["session"]["id"], self.id);
        }
        let held = fs::metadata(self.root.join("session-held-effect"))
            .unwrap()
            .len();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            fs::metadata(self.root.join("session-held-effect"))
                .unwrap()
                .len(),
            held
        );
        assert_eq!(
            self.provider.calls(),
            usize::from(self.mode != "oauth-revoke")
        );
        if self.mode == "oauth-revoke" {
            self.provider.assert_refresh(true);
        }
        assert!(!self.root.join("effects").exists());
        let database = rusqlite::Connection::open(self.state_dir.join("exports.sqlite3")).unwrap();
        let counts:(i64,i64,i64,i64)=database.query_row("SELECT (SELECT count(*) FROM worker_sessions),(SELECT count(*) FROM worker_session_turns),(SELECT coalesce(sum(reservation),0) FROM worker_sessions),(SELECT min(ack_sequence) FROM worker_sessions)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(counts, (1, 1, 0, 2));
        let calls = fs::read_to_string(self.root.join("session-calls")).unwrap();
        let calls: Vec<Value> = calls
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for method in ["initialize", "session/new", "session/prompt"] {
            assert_eq!(
                calls.iter().filter(|call| call["method"] == method).count(),
                1
            );
        }
        self.faults.assert_single_turn_no_replay();
    }
}
