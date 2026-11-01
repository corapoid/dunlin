use super::*;
use std::{fs, process::Command};
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::write(
        d.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(d.path().join("src/lib.rs"),"mod calc; use crate::calc::compute as run; pub fn wrapper()->u32 { run() } #[test] fn checks(){assert_eq!(wrapper(),1);}\n").unwrap();
    fs::write(
        d.path().join("src/calc.rs"),
        "pub fn compute()->u32 { 1 }\n",
    )
    .unwrap();
    let git = |a: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(a)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap()
                .status
                .success()
        );
    };
    git(&["init", "--template=", "-qb", "main"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["-c", "commit.gpgsign=false", "add", "."]);
    git(&[
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "-qm",
        "base",
    ]);
    d
}
#[test]
fn resolves_aliases_and_transitive_test_candidates_from_ast() {
    let d = repo();
    let index = build(d.path().to_str().unwrap(), None).unwrap();
    let symbol = index.symbols.iter().find(|s| s.name == "compute").unwrap();
    let hit = affected(&index, &symbol.id).unwrap();
    assert_eq!(hit.references.len(), 1);
    assert_eq!(hit.candidate_tests.len(), 1);
    assert_eq!(hit.candidate_tests[0].name, "checks");
    assert!(index.warnings.iter().any(|w| w.contains("NOT compiler")));
}
#[test]
fn incremental_cache_and_stale_source_verification() {
    let d = repo();
    let index = build(d.path().to_str().unwrap(), None).unwrap();
    let again = build(d.path().to_str().unwrap(), Some(&index)).unwrap();
    assert_eq!(again.parsed, 0);
    assert!(again.reused >= 2);
    fs::write(
        d.path().join("src/calc.rs"),
        "pub fn compute()->u32 { 2 }\n",
    )
    .unwrap();
    assert!(verify(&again).is_err());
    let updated = build(d.path().to_str().unwrap(), Some(&again)).unwrap();
    assert_eq!(updated.parsed, 1);
}
#[test]
fn local_shadowing_and_dynamic_methods_are_not_false_resolved_edges() {
    let d = repo();
    fs::write(d.path().join("src/lib.rs"),"fn target(){} fn foo(target: fn()){target();} struct X; impl X {fn target(&self){}} fn bar(x:X){x.target();}\n").unwrap();
    let index = build(d.path().to_str().unwrap(), None).unwrap();
    let target = index.symbols.iter().find(|s| s.name == "target").unwrap();
    assert!(affected(&index, &target.id).unwrap().references.is_empty());
    assert!(index.warnings.iter().any(|w| w.contains("dynamic")));
}
#[test]
fn added_source_inventory_and_ambiguous_definitions_invalidate_evidence() {
    let d = repo();
    let root = d.path().to_str().unwrap();
    let before = build(root, None).unwrap();
    fs::write(d.path().join("src/extra.rs"), "pub fn unseen(){}\n").unwrap();
    assert!(verify(&before).is_err());
    fs::write(
        d.path().join("src/lib.rs"),
        "#[cfg(one)] fn target(){} #[cfg(two)] fn target(){} fn caller(){target();}\n",
    )
    .unwrap();
    let index = build(root, None).unwrap();
    assert!(!index.symbols.iter().any(|s| s.name == "target"));
    assert!(index.edges.is_empty());
}
