use super::*;

#[tokio::test]
#[ignore = "requires production web build, Node, playwright-core and Chromium; run the browser sandbox suite"]
async fn production_browser_acceptance() {
    use tokio::io::AsyncWriteExt;
    let root = std::env::var_os("DUNLIN_WEB_E2E_ROOT").expect("DUNLIN_WEB_E2E_ROOT is required");
    assert!(std::path::Path::new(&root).join("index.html").is_file());
    let store = Store::open_in_memory().unwrap();
    let owner = store.create_user("browser-owner", "admin").unwrap();
    let member = store.create_user("browser-member", "member").unwrap();
    let project = store
        .create_project("Visible project", "/fixture/visible")
        .unwrap();
    let hidden = store
        .create_project("Hidden project", "/fixture/hidden")
        .unwrap();
    store.grant(&project.id, &owner.id).unwrap();
    store.grant(&hidden.id, &owner.id).unwrap();
    store.grant(&project.id, &member.id).unwrap();
    store
        .create_session(
            "member-session",
            &member.id,
            &project.id,
            "Only my session",
            "fixture",
        )
        .unwrap();
    store
        .create_session(
            "owner-session",
            &owner.id,
            &project.id,
            "Private owner session",
            "fixture",
        )
        .unwrap();
    let owner_code = store.create_pairing_code(&owner.id, 900).unwrap().0;
    let member_code = store.create_pairing_code(&member.id, 900).unwrap().0;
    let state = AppState::new(
        store,
        Config {
            web_root: Some(root.into()),
            ..Config::default()
        },
    );
    let (origin, _server) = super::live_transport::start(&state).await;
    let input = json!({"origin":origin,"ownerCode":owner_code,"memberCode":member_code,"memberId":member.id});
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/browser.mjs"
        ))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Node is required");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
        .await
        .expect("browser deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "Browser acceptance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
