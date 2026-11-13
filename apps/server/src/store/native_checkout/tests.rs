use super::*;
#[test]
fn ordinary_identity_native_epoch_settings_and_shape_are_fenced() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let s = Store::open(dir.path().join("relay.db")).unwrap();
    let u = s.create_user("native", "admin").unwrap();
    let p = s.create_project("native", root.to_str().unwrap()).unwrap();
    s.grant(&p.id, &u.id).unwrap();
    let a = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(a.root(), root.to_str().unwrap());
    assert!(a.metadata_stamp().is_none());
    assert_eq!(a.metadata_roots().iter().count(), 0);
    assert!(
        !serde_json::to_string(&a.stamp())
            .unwrap()
            .contains(dir.path().to_str().unwrap())
    );
    a.validate_filesystem().unwrap();
    let peer = Store::open(dir.path().join("relay.db")).unwrap();
    peer.revoke_grant(&p.id, &u.id).unwrap();
    peer.grant(&p.id, &u.id).unwrap();
    assert!(s.revalidate_native_checkout(&a).is_err());
    let a = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    s.save_project_settings(&p.id, "{\"a\":1}").unwrap();
    assert!(s.revalidate_native_checkout(&a).is_err());
    std::fs::write(root.join(".git"), "gitdir: /outside").unwrap();
    assert!(a.validate_filesystem().is_err());
    assert!(
        s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
            .unwrap()
            .validate_filesystem()
            .is_err()
    );
}
#[test]
fn sql_failure_is_never_treated_as_absent_metadata() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path().join("relay.db")).unwrap();
    let u = s.create_user("native", "admin").unwrap();
    let p = s
        .create_project("native", d.path().to_str().unwrap())
        .unwrap();
    s.grant(&p.id, &u.id).unwrap();
    s.connection.execute_batch("DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals;").unwrap();
    assert!(
        s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
            .is_err()
    );
}
fn linked_fixture() -> (
    tempfile::TempDir,
    Store,
    String,
    String,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("checkout");
    let common = d.path().join("repo/.git");
    let git = common.join("worktrees/native");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&git).unwrap();
    std::fs::create_dir_all(common.join("objects")).unwrap();
    std::fs::write(root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
    std::fs::write(git.join("commondir"), "../..").unwrap();
    std::fs::write(git.join("gitdir"), root.join(".git").to_str().unwrap()).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main").unwrap();
    let s = Store::open(d.path().join("relay.db")).unwrap();
    let u = s.create_user("linked", "admin").unwrap();
    let p = s.create_project("linked", root.to_str().unwrap()).unwrap();
    s.grant(&p.id, &u.id).unwrap();
    let prep = s
        .prepare_local_metadata(&p.id)
        .unwrap()
        .capture(&git, &common)
        .unwrap();
    s.commit_local_metadata(prep, None).unwrap();
    s.set_local_metadata_grant(
        &p.id,
        &u.id,
        1,
        1,
        &[MetadataPermission::Read],
        true,
        None,
        false,
    )
    .unwrap();
    (d, s, p.id, u.id, root, common)
}
#[test]
fn linked_authority_requires_own_grant_and_preserves_requested_read_only_mounts() {
    let (d, s, p, u, root, common) = linked_fixture();
    let candidate = s
        .native_checkout_candidate(&p, &u, MetadataPermission::Read)
        .unwrap();
    let a = s
        .authorize_native_checkout(candidate.validate_filesystem().unwrap())
        .unwrap();
    assert_eq!(a.root(), root.to_str().unwrap());
    assert!(a.metadata_stamp().is_some());
    assert_eq!(a.metadata_roots().iter().count(), 2);
    assert!(
        a.metadata_roots()
            .iter()
            .all(|(_, p)| p == MetadataPermission::Read)
    );
    assert!(!a.metadata_roots().is_empty());
    let mut merged = SandboxMetadataRoots::default();
    merged.extend(&a.metadata_roots()).unwrap();
    merged.extend(&a.metadata_roots()).unwrap();
    assert_eq!(merged.iter().count(), 2);
    assert!(
        s.native_checkout_candidate(&p, &u, MetadataPermission::Write)
            .is_err()
    );
    let other = s.create_user("without-metadata", "member").unwrap();
    s.grant(&p, &other.id).unwrap();
    assert!(
        s.native_checkout_candidate(&p, &other.id, MetadataPermission::Read)
            .is_err()
    );
    let peer = Store::open(d.path().join("relay.db")).unwrap();
    let validated = s
        .native_checkout_candidate(&p, &u, MetadataPermission::Read)
        .unwrap()
        .validate_filesystem()
        .unwrap();
    peer.revoke_local_metadata_grant(&p, &u, 1).unwrap();
    assert!(s.authorize_native_checkout(validated).is_err());
    assert!(s.revalidate_native_checkout(&a).is_err());
    peer.set_local_metadata_grant(
        &p,
        &u,
        1,
        1,
        &[MetadataPermission::Read, MetadataPermission::Write],
        true,
        Some(2),
        true,
    )
    .unwrap();
    assert!(s.revalidate_native_checkout(&a).is_err());
    let write = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p, &u, MetadataPermission::Write)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    assert!(
        write
            .metadata_roots()
            .iter()
            .all(|(_, p)| p == MetadataPermission::Write)
    );
    merged.extend(&write.metadata_roots()).unwrap();
    assert!(merged.iter().all(|(_, p)| p == MetadataPermission::Write));
    std::fs::rename(common.join("objects"), common.join("old-objects")).unwrap();
    std::fs::create_dir(common.join("objects")).unwrap();
    assert!(write.validate_filesystem().is_err());
}
#[cfg(unix)]
#[test]
fn original_native_alias_must_still_point_at_the_captured_checkout() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    let other = d.path().join("other");
    let alias = d.path().join("alias");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&other).unwrap();
    symlink(&root, &alias).unwrap();
    let s = Store::open(d.path().join("relay.db")).unwrap();
    let u = s.create_user("alias", "admin").unwrap();
    let p = s.create_project("alias", alias.to_str().unwrap()).unwrap();
    s.grant(&p.id, &u.id).unwrap();
    let a = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(a.root(), root.to_str().unwrap());
    std::fs::remove_file(&alias).unwrap();
    symlink(&other, &alias).unwrap();
    assert!(a.validate_filesystem().is_err());
    assert!(
        s.revalidate_native_checkout(&a).is_ok(),
        "SQL-only intentionally does not replace the mandatory filesystem check"
    );
}
#[test]
fn git_init_terminal_postcheck_is_narrow_and_does_not_refresh_authority() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let s = Store::open(d.path().join("relay.db")).unwrap();
    let u = s.create_user("init", "admin").unwrap();
    let p = s.create_project("init", root.to_str().unwrap()).unwrap();
    s.grant(&p.id, &u.id).unwrap();
    let a = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Write)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    assert!(a.validate_initialized_git().is_err());
    std::fs::create_dir_all(root.join(".git/objects")).unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
    a.validate_initialized_git().unwrap();
    s.revalidate_native_checkout_after_init(&a).unwrap();
    assert!(
        a.validate_filesystem().is_err(),
        "Git init never upgrades the stored ordinary authority"
    );
    let git = s
        .authorize_native_checkout(
            s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Write)
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    assert!(git.validate_initialized_git().is_err());
    assert!(s.revalidate_native_checkout_after_init(&git).is_err());
    s.revoke_grant(&p.id, &u.id).unwrap();
    assert!(s.revalidate_native_checkout_after_init(&a).is_err());
}
#[test]
fn batch_revalidation_checks_every_root_in_one_sql_cut() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path().join("relay.db")).unwrap();
    let u = s.create_user("batch", "admin").unwrap();
    let mut proofs = Vec::new();
    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let root = d.path().join(name);
        std::fs::create_dir(&root).unwrap();
        let p = s.create_project(name, root.to_str().unwrap()).unwrap();
        s.grant(&p.id, &u.id).unwrap();
        proofs.push(
            s.authorize_native_checkout(
                s.native_checkout_candidate(&p.id, &u.id, MetadataPermission::Read)
                    .unwrap()
                    .validate_filesystem()
                    .unwrap(),
            )
            .unwrap(),
        );
        ids.push(p.id);
    }
    s.revalidate_native_checkouts(&proofs).unwrap();
    let peer = Store::open(d.path().join("relay.db")).unwrap();
    peer.revoke_grant(&ids[1], &u.id).unwrap();
    peer.grant(&ids[1], &u.id).unwrap();
    assert!(s.revalidate_native_checkouts(&proofs).is_err());
    assert!(s.revalidate_native_checkout(&proofs[0]).is_ok());
}
