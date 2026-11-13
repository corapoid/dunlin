//! Export exact production policy bytes for a separate real macOS kernel run.
//! Linux inode witnesses authorize only this generator's read-only fixture view;
//! they are never represented as macOS authority or a portable filesystem proof.
use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema_version: u16,
    fixture_root: String,
    home: String,
    checkout: String,
    common: String,
    git: String,
    source: String,
    other: String,
    db: String,
}
#[test]
#[ignore = "requires --suite macos-profile with a read-only same-path host fixture"]
fn export_production_macos_metadata_profiles() {
    let root =
        std::env::var("DUNLIN_MACOS_PROFILE_FIXTURE").expect("explicit host fixture required");
    let root = PathBuf::from(root);
    assert!(root.is_absolute());
    assert_eq!(root.canonicalize().unwrap(), root);
    let raw = std::fs::read(root.join("fixture.json")).unwrap();
    assert!(raw.len() <= 16 * 1024);
    let input: Input = serde_json::from_slice(&raw).unwrap();
    assert_eq!(input.schema_version, 1);
    assert_eq!(Path::new(&input.fixture_root), root.as_path());
    assert_eq!(std::env::var("HOME").unwrap(), input.home);
    for path in [
        &input.home,
        &input.checkout,
        &input.common,
        &input.git,
        &input.source,
        &input.other,
        &input.db,
    ] {
        let path = Path::new(path);
        assert!(path.is_absolute() && path.starts_with(&root) && path != root.as_path());
        assert_eq!(path.canonicalize().unwrap(), path);
    }
    let config = crate::config::Config {
        db: input.db.clone().into(),
        sandbox: true,
        ..Default::default()
    };
    let settings: Value = serde_json::from_slice(
        &std::fs::read(crate::management::root(&config).join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["isolated_agents"], true,
        "Mac policy selection must agree with the generator"
    );
    assert!(crate::management::isolated_agents(&config));
    let store = crate::store::Store::open_in_memory().unwrap();
    let user = store
        .create_user("profile-export-fixture", "member")
        .unwrap();
    let project = store
        .create_project("approved linked checkout", &input.checkout)
        .unwrap();
    store.grant(&project.id, &user.id).unwrap();
    for (name, path) in [
        ("private source workspace", &input.source),
        ("private other workspace", &input.other),
    ] {
        let hidden = store.create_project(name, path).unwrap();
        assert!(
            store
                .project_for_user(&hidden.id, &user.id)
                .unwrap()
                .is_none()
        );
    }
    let prepared = store
        .prepare_local_metadata(&project.id)
        .unwrap()
        .capture(Path::new(&input.git), Path::new(&input.common))
        .unwrap();
    store.commit_local_metadata(prepared, None).unwrap();
    let process_home = crate::management::agent_home(&config, &user.id);
    assert!(process_home.starts_with(root.join("server/agent-homes")));
    let other_projects = store
        .list_projects()
        .unwrap()
        .into_iter()
        .filter(|p| p.id != project.id)
        .map(|p| p.path)
        .collect::<Vec<_>>();
    let mut profiles = Vec::new();
    for (permission, expected) in [
        (MetadataPermission::Read, None),
        (MetadataPermission::Write, Some(1)),
    ] {
        let permissions = if permission == MetadataPermission::Read {
            vec![MetadataPermission::Read]
        } else {
            vec![MetadataPermission::Read, MetadataPermission::Write]
        };
        store
            .set_local_metadata_grant(
                &project.id,
                &user.id,
                1,
                1,
                &permissions,
                true,
                expected,
                permission == MetadataPermission::Write,
            )
            .unwrap();
        let validated = store
            .native_checkout_candidate(&project.id, &user.id, permission)
            .unwrap()
            .validate_filesystem()
            .unwrap();
        let authority = store.authorize_native_checkout(validated).unwrap();
        authority.validate_filesystem().unwrap();
        store.revalidate_native_checkout(&authority).unwrap();
        let metadata = authority.metadata_roots();
        assert!(!metadata.is_empty() && metadata.iter().all(|(_, p)| p == permission));
        assert!(metadata_roots_safe(&config, &metadata));
        assert!(!metadata.iter().any(|(root, _)| {
            other_projects
                .iter()
                .any(|p| Path::new(p).starts_with(root))
        }));
        let profile = macos_harness_profile(
            &config,
            "/usr/bin/python3",
            &input.checkout,
            &[],
            &other_projects,
            &process_home,
            &[],
            &metadata,
            true,
        );
        assert!(profile.len() <= 128 * 1024);
        let mode = if permission == MetadataPermission::Read {
            "read"
        } else {
            "write"
        };
        profiles.push(json!({"permission":mode,"profile_sha256":hex::encode(Sha256::digest(profile.as_bytes())),"profile":profile,
            "cwd":input.checkout,"args":[input.common,input.git,format!("{}/source-only-secret",input.source),format!("{}/other-project-secret",input.other),input.db,mode]}));
    }
    let bundle = json!({"schema_version":1,"generator":"production_macos_harness_profile","authority_platform":"linux-fixture-only",
        "fixture_root":input.fixture_root,"profile_home":input.home,"process_home":process_home,
        "program":"/usr/bin/python3","python_args":["-I","-S"],"probe":kernel_probe_code::PROBE,"control":kernel_probe_code::CONTROL,"profiles":profiles});
    let encoded = serde_json::to_string(&bundle).unwrap();
    assert!(encoded.len() <= 384 * 1024);
    println!("\nDUNLIN_MACOS_PROFILE_BUNDLE:{encoded}");
}
