//! Validate before initializing GPUI, storage, logging or any worker thread.
use std::{ffi::OsString, path::Path};

#[derive(Debug, serde::Serialize)]
pub(crate) struct NativeSmoke {
    pub flag: &'static str,
    pub fixture: Option<&'static str>,
    pub light_narrow: bool,
    pub timeout_secs: u64,
}

// The QA runner discovers this registry from the built debug binary. Adding a
// scenario here makes it part of the native suite, not an undocumented opt-in.
pub(crate) const NATIVE_SMOKES: &[NativeSmoke] = &[
    NativeSmoke {
        flag: "--surfaces-smoke",
        fixture: None,
        light_narrow: false,
        timeout_secs: 60,
    },
    NativeSmoke {
        flag: "--coordination-smoke",
        fixture: Some("coordination"),
        light_narrow: false,
        timeout_secs: 90,
    },
    NativeSmoke {
        flag: "--controls-smoke",
        fixture: None,
        light_narrow: false,
        timeout_secs: 60,
    },
    NativeSmoke {
        flag: "--settings-smoke",
        fixture: Some("settings"),
        light_narrow: false,
        timeout_secs: 120,
    },
    NativeSmoke {
        flag: "--workspace-ui-smoke",
        fixture: Some("workspace-ui"),
        light_narrow: false,
        timeout_secs: 150,
    },
    NativeSmoke {
        flag: "--project-import-smoke",
        fixture: Some("project-import"),
        light_narrow: false,
        timeout_secs: 120,
    },
    NativeSmoke {
        flag: "--session-import-smoke",
        fixture: Some("session-import"),
        light_narrow: false,
        timeout_secs: 120,
    },
    NativeSmoke {
        flag: "--git-smoke",
        fixture: Some("git"),
        light_narrow: true,
        timeout_secs: 100,
    },
    NativeSmoke {
        flag: "--workbench-smoke",
        fixture: Some("git"),
        light_narrow: true,
        timeout_secs: 100,
    },
    NativeSmoke {
        flag: "--markdown-smoke",
        fixture: Some("markdown"),
        light_narrow: true,
        timeout_secs: 100,
    },
    NativeSmoke {
        flag: "--glass-smoke",
        fixture: Some("glass"),
        light_narrow: false,
        timeout_secs: 60,
    },
    NativeSmoke {
        flag: "--onboarding-smoke",
        fixture: Some("onboarding"),
        light_narrow: false,
        timeout_secs: 60,
    },
    NativeSmoke {
        flag: "--save-smoke",
        fixture: Some("save"),
        light_narrow: false,
        timeout_secs: 60,
    },
];

pub(crate) fn validate(args: &[OsString], debug: bool) -> Result<(), &'static str> {
    let Some(first) = args.first() else {
        return Ok(());
    };
    let flag = first.to_str().ok_or("Invalid command-line argument")?;
    // Older LaunchServices versions may supply a process serial number.
    if args.len() == 1
        && flag
            .strip_prefix("-psn_")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'_'))
    {
        return Ok(());
    }
    if matches!(
        flag,
        "--help"
            | "-h"
            | "--version"
            | "-V"
            | "--syntax-notices"
            | "--mcp-vault"
            | "--mcp-coordination"
            | "--vault-keychain-smoke"
    ) {
        return if args.len() == 1 {
            Ok(())
        } else {
            Err("Unexpected extra arguments")
        };
    }
    if debug && flag == "--list-native-smokes" {
        return if args.len() == 1 {
            Ok(())
        } else {
            Err("Unexpected extra arguments")
        };
    }
    if debug && flag == "--governance-smoke" {
        return if args.len() <= 2
            && args
                .get(1)
                .is_none_or(|p| !p.to_string_lossy().starts_with('-'))
        {
            Ok(())
        } else {
            Err("Expected an optional plugin directory")
        };
    }
    if debug && let Some(smoke) = NATIVE_SMOKES.iter().find(|s| s.flag == flag) {
        let Some(fixture) = smoke.fixture else {
            return if args.len() == 1 {
                Ok(())
            } else {
                Err("Unexpected extra arguments")
            };
        };
        let path = args
            .get(1)
            .map(Path::new)
            .ok_or("Expected a fresh absolute fixture directory")?;
        let prefix = format!("dunlin-{fixture}-smoke-");
        if !path.is_absolute()
            || !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        {
            return Err("Expected a fresh absolute fixture directory with the scenario prefix");
        }
        if args.len() == 2
            || (args.len() == 3
                && ((smoke.light_narrow && args[2] == "--light-narrow")
                    || (flag == "--glass-smoke" && args[2] == "--settings-close-only")))
        {
            return Ok(());
        }
        return Err("Unexpected native smoke arguments");
    }
    Err("Unknown or unavailable command-line option; use --help")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn unknown_removed_and_release_only_options_fail_closed() {
        for debug in [true, false] {
            for values in [
                vec!["--github-smoke"],
                vec!["--typo"],
                vec!["file.txt"],
                vec!["--syntax-notices", "--git-smoke"],
            ] {
                assert!(validate(&args(&values), debug).is_err());
            }
        }
        assert!(validate(&[], false).is_ok());
        assert!(validate(&args(&["-psn_0_12345"]), false).is_ok());
        assert!(validate(&args(&["--help"]), false).is_ok());
        assert!(validate(&args(&["--list-native-smokes"]), false).is_err());
    }

    #[test]
    fn every_registered_smoke_accepts_only_its_own_fixture_and_options() {
        let mut flags = std::collections::HashSet::new();
        for smoke in NATIVE_SMOKES {
            assert!(flags.insert(smoke.flag));
            let mut values = vec![OsString::from(smoke.flag)];
            if let Some(fixture) = smoke.fixture {
                assert!(validate(&values, true).is_err());
                values.push(format!("/fixture/dunlin-{fixture}-smoke-test").into());
                assert!(validate(&args(&[smoke.flag, "relative-path"]), true).is_err());
                assert!(validate(&args(&[smoke.flag, "/fixture/wrong-prefix"]), true).is_err());
            }
            assert!(validate(&values, true).is_ok(), "{}", smoke.flag);
            assert!(validate(&values, false).is_err());
            values.push("--light-narrow".into());
            assert_eq!(validate(&values, true).is_ok(), smoke.light_narrow);
            values.push("--extra".into());
            assert!(validate(&values, true).is_err());
        }
    }
}
