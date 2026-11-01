//! Same-commit comparison worktrees. Never starts agents, pushes, merges, copies
//! dirty source buffers or deletes partially created trees on failure.
use crate::git::operations::{self, Action, Plan};
use crate::task_review::{self, Checkpoint};
use serde::{Deserialize, Serialize};
pub struct Prepared {
    pub id: String,
    pub root: String,
    pub head: String,
    pub paths: [String; 2],
    source: Plan,
    add: [Plan; 2],
    created: std::time::Instant,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trees {
    pub id: String,
    pub root: String,
    pub head: String,
    pub paths: Vec<String>,
    pub baselines: Vec<Checkpoint>,
    pub error: Option<String>,
}
pub fn prepare(root: &str) -> Result<Prepared, String> {
    let source = operations::preflight(
        root,
        Action::Merge {
            revision: "HEAD".into(),
        },
    )?;
    let head = source
        .head
        .clone()
        .ok_or("Comparison needs a committed HEAD")?;
    let id = uuid::Uuid::now_v7().to_string();
    let a = operations::preflight(
        root,
        Action::WorktreeAdd {
            name: format!("compare-{id}-a"),
            revision: head.clone(),
        },
    )?;
    let b = operations::preflight(
        root,
        Action::WorktreeAdd {
            name: format!("compare-{id}-b"),
            revision: head.clone(),
        },
    )?;
    if a.fingerprint != source.fingerprint || b.fingerprint != source.fingerprint {
        return Err("Source changed during pair preflight".into());
    }
    let paths = [
        a.worktree_target.clone().ok_or("Missing A destination")?,
        b.worktree_target.clone().ok_or("Missing B destination")?,
    ];
    Ok(Prepared {
        id,
        root: source.root.clone(),
        head,
        paths,
        source,
        add: [a, b],
        created: std::time::Instant::now(),
    })
}
pub fn validate_tree(path: &str, head: &str) -> Result<(), String> {
    let plan = operations::preflight(
        path,
        Action::Merge {
            revision: head.into(),
        },
    )?;
    if plan.head.as_deref() != Some(head) {
        return Err("Comparison tree HEAD no longer matches the approved baseline".into());
    }
    if operations::read(path, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim() != "HEAD" {
        return Err("Comparison tree is no longer detached; no agent start authorized".into());
    }
    if !operations::read(
        path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?
    .is_empty()
    {
        return Err("Comparison tree has dirty/untracked/ignored content. No agent is authorized until it is explicitly reviewed and cleaned.".into());
    }
    Ok(())
}
pub fn create(plan: Prepared) -> Trees {
    let mut result = Trees {
        id: plan.id,
        root: plan.root.clone(),
        head: plan.head.clone(),
        paths: Vec::new(),
        baselines: Vec::new(),
        error: None,
    };
    let work = (|| {
        if plan.created.elapsed() > std::time::Duration::from_secs(300) {
            return Err("Worktree-pair confirmation expired".into());
        }
        for (i, add) in plan.add.iter().enumerate() {
            if i == 1 {
                operations::unchanged_except_added_worktrees(&plan.source, &[&plan.paths[0]])?;
            }
            let fresh = if i == 0 {
                add.clone()
            } else {
                operations::preflight(&plan.root, add.action.clone())?
            };
            let outcome = operations::execute(&plan.root, &fresh)?;
            // Even a failed post-checkout hook may leave an actual tree. Retain
            // and report it; never hide or automatically remove such content.
            if std::path::Path::new(&plan.paths[i]).exists() {
                result.paths.push(plan.paths[i].clone());
            }
            if !outcome.success {
                return Err("Git refused a comparison checkout or a hook failed. Existing trees are retained; no agent started.".into());
            }
            validate_tree(&plan.paths[i], &plan.head)?;
            result.baselines.push(task_review::capture(
                &plan.paths[i],
                "Shared comparison baseline",
            )?);
        }
        // A second hook can alter the source or the already validated A tree.
        operations::unchanged_except_added_worktrees(
            &plan.source,
            &[&plan.paths[0], &plan.paths[1]],
        )?;
        for path in &plan.paths {
            validate_tree(path, &plan.head)?;
        }
        Ok::<_, String>(())
    })();
    result.error = work.err();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let git = |a: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(d.path())
                    .args(a)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        git(&["init", "--template=", "-qb", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["config", "core.hooksPath", "/dev/null"]);
        fs::write(d.path().join("file"), "same input\n").unwrap();
        git(&["add", "file"]);
        git(&["commit", "-qm", "base"]);
        d
    }
    #[test]
    fn pair_is_same_commit_detached_and_source_stays_untouched() {
        let d = repo();
        let p = prepare(d.path().to_str().unwrap()).unwrap();
        let paths = p.paths.clone();
        assert!(paths.iter().all(|p| !std::path::Path::new(p).exists()));
        let r = create(p);
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.baselines.len(), 2);
        assert_eq!(r.paths.len(), 2);
        assert!(r.baselines.iter().all(|b| b.head == r.head));
        for path in &paths {
            assert_eq!(
                fs::read_to_string(std::path::Path::new(path).join("file")).unwrap(),
                "same input\n"
            );
        }
        assert_eq!(
            fs::read_to_string(d.path().join("file")).unwrap(),
            "same input\n"
        );
    }
    #[test]
    fn dirty_source_and_stale_approval_never_start_checkout() {
        let d = repo();
        let root = d.path().to_str().unwrap();
        let p = prepare(root).unwrap();
        fs::write(d.path().join("file"), "user edit").unwrap();
        assert!(prepare(root).is_err());
        let r = create(p);
        assert!(r.error.is_some());
        assert!(r.paths.is_empty());
        assert_eq!(
            fs::read_to_string(d.path().join("file")).unwrap(),
            "user edit"
        );
    }
    #[cfg(unix)]
    #[test]
    fn post_checkout_source_mutation_retains_first_tree_and_refuses_second() {
        use std::os::unix::fs::PermissionsExt;
        let d = repo();
        let hooks = d.path().join(".git/hooks");
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\nprintf 'hook edit\\n' > '{}'\n",
                d.path().join("file").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(["config", "core.hooksPath"])
                .arg(&hooks)
                .status()
                .unwrap()
                .success()
        );
        let plan = prepare(d.path().to_str().unwrap()).unwrap();
        let paths = plan.paths.clone();
        let result = create(plan);
        assert!(result.error.is_some(), "{:?}", result);
        assert_eq!(result.paths, vec![paths[0].clone()]);
        assert!(!std::path::Path::new(&paths[1]).exists());
        assert_eq!(
            fs::read_to_string(d.path().join("file")).unwrap(),
            "hook edit\n"
        );
    }
    #[cfg(unix)]
    #[test]
    fn second_checkout_hook_cannot_mutate_source_and_report_a_valid_pair() {
        use std::os::unix::fs::PermissionsExt;
        let d = repo();
        let hooks = d.path().join(".git/hooks");
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\ncase \"$PWD\" in *-b) printf 'second hook edit\\n' > '{}' ;; esac\n",
                d.path().join("file").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(d.path())
                .args(["config", "core.hooksPath"])
                .arg(&hooks)
                .status()
                .unwrap()
                .success()
        );
        let result = create(prepare(d.path().to_str().unwrap()).unwrap());
        assert_eq!(result.paths.len(), 2);
        assert!(result.error.is_some(), "{result:?}");
    }
}
