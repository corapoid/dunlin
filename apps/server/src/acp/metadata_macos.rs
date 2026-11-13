//! Real seatbelt regression. This regular macOS test is part of the existing
//! cargo-test CI gate; it neither skips a missing kernel adapter nor falls back.
use super::*;

use super::kernel_probe_code::PROBE;

async fn probe(fixture: &Fixture, other: &Path, source: &Path, permission: MetadataPermission) {
    let mode = if permission == MetadataPermission::Write {
        "write"
    } else {
        "read"
    };
    let root = fixture.root.canonicalize().unwrap();
    let source_secret = source.join("source-only-secret");
    let other_secret = other.join("other-project-secret");
    let mut config = (*fixture.state.config).clone();
    config.db = config.db.canonicalize().unwrap();
    assert!(config.sandbox);
    for file in [&source_secret, &other_secret, &config.db] {
        assert!(
            file.is_file(),
            "Protected probe target must exist: {file:?}"
        );
    }
    let metadata = fixture.authority(permission).metadata_roots();
    assert!(!metadata.is_empty());
    assert!(metadata.iter().all(|(_, p)| p == permission));
    let args = vec![
        "-I".into(),
        "-S".into(),
        "-c".into(),
        PROBE.into(),
        fixture.common.to_str().unwrap().into(),
        fixture.git.canonicalize().unwrap().to_str().unwrap().into(),
        source_secret.to_str().unwrap().into(),
        other_secret.to_str().unwrap().into(),
        config.db.to_str().unwrap().into(),
        mode.into(),
    ];
    let registered = fixture.state.store.lock().unwrap().list_projects().unwrap();
    let other_projects = registered
        .into_iter()
        .filter(|p| p.id != fixture.project.id)
        .map(|p| p.path)
        .collect::<Vec<_>>();
    assert!(other_projects.iter().any(|p| Path::new(p) == source));
    assert!(other_projects.iter().any(|p| Path::new(p) == other));
    let mut command = harness_command_with_metadata(
        &config,
        "/usr/bin/python3",
        &args,
        root.to_str().unwrap(),
        &[],
        &other_projects,
        &ProjectSettings::default(),
        &fixture.user.id,
        &metadata,
    );
    assert_eq!(
        command.as_std().get_program(),
        "/usr/bin/sandbox-exec",
        "This test requires the production macOS kernel adapter; no unsandboxed fallback"
    );
    command
        .current_dir(&root)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .expect("macOS metadata kernel probe deadline")
        .expect("production sandbox-exec must spawn");
    assert!(
        output.status.success(),
        "Actual macOS {mode} metadata kernel probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        receipt,
        json!({"kernel":"seatbelt","metadata":mode,"protected_files":3})
    );
    for metadata in [&fixture.common, &fixture.git] {
        if permission == MetadataPermission::Read {
            assert_eq!(
                std::fs::read_to_string(metadata.join("kernel-existing")).unwrap(),
                "original"
            );
            assert!(!metadata.join("kernel-created").exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(metadata.join("kernel-existing")).unwrap(),
                "originalwrite"
            );
            assert_eq!(
                std::fs::read_to_string(metadata.join("kernel-created")).unwrap(),
                "write"
            );
        }
    }
}

#[tokio::test]
async fn macos_metadata_own_read_is_kernel_read_only_and_own_write_is_kernel_writable() {
    let fixture = Fixture::new();
    let source = fixture.source.canonicalize().unwrap();
    let other = fixture._dir.path().join("other-project");
    std::fs::create_dir(&other).unwrap();
    let other = other.canonicalize().unwrap();
    std::fs::write(other.join("other-project-secret"), "fixture-only-private").unwrap();
    {
        let store = fixture.state.store.lock().unwrap();
        for (name, path) in [
            ("private source workspace", &source),
            ("other private project", &other),
        ] {
            let project = store.create_project(name, path.to_str().unwrap()).unwrap();
            assert!(
                store
                    .project_for_user(&project.id, &fixture.user.id)
                    .unwrap()
                    .is_none()
            );
        }
    }
    for metadata in [&fixture.common, &fixture.git] {
        std::fs::write(metadata.join("kernel-existing"), "original").unwrap();
    }
    fixture.grant(&[MetadataPermission::Read], None);
    probe(&fixture, &other, &source, MetadataPermission::Read).await;
    fixture.grant(
        &[MetadataPermission::Read, MetadataPermission::Write],
        Some(1),
    );
    probe(&fixture, &other, &source, MetadataPermission::Write).await;
    assert_eq!(
        std::fs::read_to_string(source.join("source-only-secret")).unwrap(),
        "private-source-workspace"
    );
    assert_eq!(
        std::fs::read_to_string(other.join("other-project-secret")).unwrap(),
        "fixture-only-private"
    );
    // Kernel denial attempts must leave the live fixture database usable.
    assert!(
        fixture
            .state
            .store
            .lock()
            .unwrap()
            .find_user(&fixture.user.id)
            .unwrap()
            .is_some()
    );
}
