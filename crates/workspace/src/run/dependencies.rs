//! Read-only Node dependency presence checks, using the same manifests as Run.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    pub candidate: RunCandidate,
    pub installed: bool,
}

pub(super) fn install_dir(root: &Path, dir: &Path) -> PathBuf {
    dir.ancestors()
        .take_while(|parent| parent.starts_with(root))
        .find(|parent| {
            parent.join("pnpm-workspace.yaml").is_file()
                || read_limited(&parent.join("package.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .is_some_and(|value| value.get("workspaces").is_some())
        })
        .unwrap_or(dir)
        .to_path_buf()
}

fn package_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    match parts.as_slice() {
        [name] => valid(name),
        [scope, name] => scope.strip_prefix('@').is_some_and(valid) && valid(name),
        _ => false,
    }
}

pub(super) fn detect_node(
    root: &Path,
    dir: &Path,
    value: &serde_json::Value,
    detection: &mut Detection,
) {
    let names: BTreeSet<_> = ["dependencies", "devDependencies"]
        .into_iter()
        .filter_map(|key| value.get(key).and_then(serde_json::Value::as_object))
        .flat_map(|deps| deps.keys())
        .collect();
    if names.is_empty() {
        return;
    }
    let owner = install_dir(root, dir);
    // shortcut: check package presence, not semver or PnP archive integrity; use manager validation for stale installs.
    let pnp = owner.join(".pnp.cjs").is_file() || owner.join(".pnp.js").is_file();
    let installed = names.iter().all(|name| {
        package_name(name)
            && (pnp
                || dir
                    .ancestors()
                    .take_while(|parent| parent.starts_with(root))
                    .any(|parent| {
                        parent
                            .join("node_modules")
                            .join(name)
                            .join("package.json")
                            .is_file()
                    }))
    });
    let manager = package_manager(&owner);
    let Some(candidate) = candidate(
        root,
        &owner.join("package.json"),
        &owner,
        "dependency",
        format!("{manager} install"),
        manager,
        vec!["install".into()],
    ) else {
        return;
    };
    if let Some(existing) = detection
        .dependencies
        .iter_mut()
        .find(|dep| dep.candidate.id == candidate.id)
    {
        existing.installed &= installed;
    } else {
        detection.dependencies.push(Dependency {
            candidate,
            installed,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::fixture;

    #[test]
    fn missing_and_partial_installs_are_not_ready_even_without_run_scripts() {
        let root = fixture("dependencies");
        fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"react":"*","@scope/pkg":"*"}}"#,
        )
        .unwrap();
        let check = || detect(root.to_str().unwrap()).dependencies.remove(0);
        assert!(!check().installed);
        fs::create_dir_all(root.join("node_modules/react")).unwrap();
        fs::write(root.join("node_modules/react/package.json"), "{}").unwrap();
        assert!(!check().installed);
        fs::create_dir_all(root.join("node_modules/@scope/pkg")).unwrap();
        fs::write(root.join("node_modules/@scope/pkg/package.json"), "{}").unwrap();
        assert!(check().installed);
        assert_eq!(check().candidate.command_line(), "npm install");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_packages_share_one_installer_and_hoisted_dependencies() {
        let root = fixture("workspace-dependencies");
        fs::write(
            root.join("package.json"),
            r#"{"workspaces":["packages/*"],"devDependencies":{"typescript":"*"}}"#,
        )
        .unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        fs::create_dir_all(root.join("packages/web")).unwrap();
        fs::write(
            root.join("packages/web/package.json"),
            r#"{"scripts":{"dev":"vite"},"dependencies":{"react":"*"}}"#,
        )
        .unwrap();
        let detection = detect(root.to_str().unwrap());
        assert_eq!(detection.dependencies.len(), 1);
        assert_eq!(detection.dependencies[0].candidate.cwd, ".");
        assert_eq!(
            detection.dependencies[0].candidate.command_line(),
            "pnpm install"
        );
        assert!(!detection.dependencies[0].installed);
        assert_eq!(detection.candidates[0].program, "pnpm");
        for name in ["typescript", "react"] {
            fs::create_dir_all(root.join("node_modules").join(name)).unwrap();
            fs::write(
                root.join("node_modules").join(name).join("package.json"),
                "{}",
            )
            .unwrap();
        }
        assert!(detect(root.to_str().unwrap()).dependencies[0].installed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_each_lockfile_and_pnp_without_executing_a_manager() {
        for (lock, manager) in [
            ("package-lock.json", "npm"),
            ("yarn.lock", "yarn"),
            ("pnpm-lock.yaml", "pnpm"),
            ("bun.lock", "bun"),
        ] {
            let root = fixture(manager);
            fs::write(
                root.join("package.json"),
                r#"{"devDependencies":{"vite":"*"}}"#,
            )
            .unwrap();
            fs::write(root.join(lock), "").unwrap();
            let detection = detect(root.to_str().unwrap());
            assert_eq!(detection.dependencies[0].candidate.program, manager);
            assert!(!detection.dependencies[0].installed);
            if manager == "yarn" {
                fs::write(root.join(".pnp.cjs"), "").unwrap();
                assert!(detect(root.to_str().unwrap()).dependencies[0].installed);
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn declared_package_manager_takes_precedence_and_empty_projects_need_no_install() {
        let root = fixture("declared-manager");
        fs::write(
            root.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0","devDependencies":{"vite":"*"}}"#,
        )
        .unwrap();
        fs::write(root.join("yarn.lock"), "").unwrap();
        assert_eq!(
            detect(root.to_str().unwrap()).dependencies[0]
                .candidate
                .program,
            "pnpm"
        );
        fs::write(
            root.join("package.json"),
            r#"{"scripts":{"test":"echo test"}}"#,
        )
        .unwrap();
        assert!(detect(root.to_str().unwrap()).dependencies.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dependency_names_cannot_escape_node_modules() {
        for name in [
            "../secret",
            "@scope/../../secret",
            "/tmp/pkg",
            "a/b/c",
            "@/pkg",
            "a\\b",
        ] {
            assert!(!package_name(name));
        }
        for name in ["react", "@scope/pkg", "some-package.js"] {
            assert!(package_name(name));
        }
    }
}
