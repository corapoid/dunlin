use super::*;
fn plan(path: &Path, cwd: &Path) -> RecipePlan {
    RecipePlan {
        program: path.to_str().unwrap().into(),
        args: vec![],
        env_var: "DUNLIN_SECRET".into(),
        secret: Zeroizing::new("fixture-secret".into()),
        cwd: cwd.into(),
        timeout: Duration::from_secs(2),
        program_hash: Some(program_hash(path).unwrap()),
    }
}
#[test]
fn captured_program_is_sealed_and_independent_of_later_path_replacement() {
    let programs = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let path = programs.path().join("program");
    let bytes = b"#!/bin/sh\necho original\n";
    std::fs::write(&path, bytes).unwrap();
    let request = plan(&path, cwd.path());
    let access = LiveAccess::new(Instant::now() + Duration::from_secs(2), || true);
    let mut image = snapshot(&request, &access).unwrap();
    std::fs::write(&path, b"#!/bin/sh\necho replaced\n").unwrap();
    let mut captured = Vec::new();
    image.read_to_end(&mut captured).unwrap();
    assert_eq!(captured, bytes);
    assert!(image.write_all(b"changed").is_err());
    assert!(image.set_len(1).is_err());
    assert!(snapshot(&request, &access).is_err());
}
#[test]
fn program_capture_refuses_checkout_images_and_revoked_authority() {
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().join("program");
    std::fs::write(&path, "fixed").unwrap();
    let request = plan(&path, cwd.path());
    let access = LiveAccess::new(Instant::now() + Duration::from_secs(2), || true);
    assert!(snapshot(&request, &access).is_err());
    let other = tempfile::tempdir().unwrap();
    let request = plan(&path, other.path());
    access.revoke();
    assert!(snapshot(&request, &access).is_err());
}
