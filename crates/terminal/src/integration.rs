//! Opt-in shell integration (OSC 133 / OSC 7) and per-process wrapper files.
//!
//! The wrappers live in a private temporary directory and never modify the
//! user's dotfiles. zsh is kept as a login shell; bash is started as an
//! interactive shell with an explicit `--rcfile` because a login bash ignores
//! `--rcfile`. Other shells are left untouched.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integration {
    /// Extra arguments prepended to the shell invocation.
    pub args: Vec<String>,
    /// Internal environment for the wrapper. These are applied after the
    /// project environment filter so they cannot be shadowed from project
    /// settings.
    pub env: Vec<(String, String)>,
    /// The user's original zsh startup directory. Used for the history file so
    /// tabs do not share the temporary wrapper directory.
    pub original_zdotdir: Option<String>,
}

pub fn prepare(shell: &str, home_override: Option<&str>) -> Option<Integration> {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dir = wrapper_dir()?;
    if name.ends_with("zsh") {
        let original = home_override.map(str::to_string).or_else(|| {
            std::env::var("ZDOTDIR")
                .ok()
                .filter(|value| !value.is_empty())
        });
        Some(Integration {
            args: vec!["-l".into()],
            env: vec![
                ("ZDOTDIR".into(), dir.to_string_lossy().to_string()),
                (
                    "_DUNLIN_WRAPPER_DIR".into(),
                    dir.to_string_lossy().to_string(),
                ),
                (
                    "_DUNLIN_USER_ZDOTDIR".into(),
                    original.clone().unwrap_or_default(),
                ),
                ("DUNLIN_SHELL_INTEGRATION".into(), "1".to_string()),
            ],
            original_zdotdir: original,
        })
    } else if name.ends_with("bash") {
        Some(Integration {
            args: vec![
                "--rcfile".into(),
                dir.join("bashrc").to_string_lossy().to_string(),
                "-i".into(),
            ],
            env: vec![("DUNLIN_SHELL_INTEGRATION".into(), "1".to_string())],
            original_zdotdir: None,
        })
    } else {
        None
    }
}

fn wrapper_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("dunlin-shell-{}-{stamp}", std::process::id()));
        if fs::create_dir_all(&dir).is_err() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        let zshenv = dir.join(".zshenv");
        let zprofile = dir.join(".zprofile");
        let zshrc = dir.join(".zshrc");
        let bashrc = dir.join("bashrc");
        for (path, contents) in [
            (zshenv, ZSH_ENV),
            (zprofile, ZSH_PROFILE),
            (zshrc, ZSH_RC),
            (bashrc, BASH_RC),
        ] {
            if fs::write(&path, contents).is_err() {
                let _ = fs::remove_dir_all(&dir);
                return None;
            }
        }
        Some(dir)
    })
    .clone()
}

pub fn quote(value: &str) -> String {
    if value.is_empty() {
        return "''".into();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

const ZSH_ENV: &str = r#"# Dunlin shell integration (zshenv).
if [[ -n "${_DUNLIN_USER_ZDOTDIR-}" ]]; then
  typeset -g _DUNLIN_ORIG_ZDOTDIR="$_DUNLIN_USER_ZDOTDIR"
else
  typeset -g _DUNLIN_ORIG_ZDOTDIR="$HOME"
fi
if [[ -f "$_DUNLIN_ORIG_ZDOTDIR/.zshenv" ]]; then
  source "$_DUNLIN_ORIG_ZDOTDIR/.zshenv"
fi
# Keep the remaining startup files inside the wrapper, then restore ZDOTDIR.
ZDOTDIR="$_DUNLIN_WRAPPER_DIR"
export ZDOTDIR
"#;

const ZSH_PROFILE: &str = r#"# Dunlin shell integration (zprofile).
if [[ -f "$_DUNLIN_ORIG_ZDOTDIR/.zprofile" ]]; then
  source "$_DUNLIN_ORIG_ZDOTDIR/.zprofile"
fi
"#;

const ZSH_RC: &str = r#"# Dunlin shell integration (zshrc).
# Restore the real ZDOTDIR before sourcing the user's rc so plugins and
# compdump/history files keep using the user's original location.
ZDOTDIR="$_DUNLIN_ORIG_ZDOTDIR"
export ZDOTDIR
if [[ -f "$ZDOTDIR/.zshrc" ]]; then
  source "$ZDOTDIR/.zshrc"
fi

__dunlin_precmd() {
  local _dunlin_status=$?
  printf '\033]133;D;%s\a' "$_dunlin_status"
  printf '\033]7;file://%s%s\a' "${HOST-}" "$PWD"
  printf '\033]133;A\a'
  return $_dunlin_status
}
__dunlin_preexec() {
  printf '\033]133;C\a'
}
__dunlin_prompt_mark() {
  printf '\033]133;B\a'
}
precmd_functions=(__dunlin_precmd ${precmd_functions:#__dunlin_precmd})
preexec_functions=(__dunlin_preexec ${preexec_functions:#__dunlin_preexec})
PS1="${PS1}%{$(__dunlin_prompt_mark)%}"
"#;

const BASH_RC: &str = r#"# Dunlin shell integration (bashrc).
# `--rcfile` starts an interactive non-login shell, so mirror the login
# profile files first and then the regular rc.
if [[ -z "${_DUNLIN_PROFILE_LOADED-}" ]]; then
  export _DUNLIN_PROFILE_LOADED=1
  for __dunlin_profile in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [[ -f "$__dunlin_profile" ]]; then
      source "$__dunlin_profile"
      break
    fi
  done
  unset __dunlin_profile
fi
if [[ -n "${HOME-}" && -f "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi
__DUNLIN_AT_PROMPT=0
__dunlin_capture_status() {
  __DUNLIN_STATUS=$?
}
__dunlin_precmd() {
  __DUNLIN_AT_PROMPT=0
  local _dunlin_status=${__DUNLIN_STATUS:-0}
  printf '\033]133;D;%s\a' "$_dunlin_status"
  printf '\033]7;file://%s%s\a' "${HOSTNAME-}" "$PWD"
  printf '\033]133;A\a'
  return $_dunlin_status
}
__dunlin_prompt_ready() {
  __DUNLIN_AT_PROMPT=1
}
__dunlin_preexec() {
  [[ "${__DUNLIN_AT_PROMPT:-0}" == 1 ]] || return
  __DUNLIN_AT_PROMPT=0
  [[ -n "${COMP_LINE-}" ]] && return
  printf '\033]133;C\a'
}
PROMPT_COMMAND="__dunlin_capture_status; __dunlin_precmd; ${PROMPT_COMMAND:+$PROMPT_COMMAND; }__dunlin_prompt_ready"
trap '__dunlin_preexec' DEBUG
PS1="${PS1}\[\033]133;B\a\]"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_remote_paths_without_local_shell_expansion() {
        assert_eq!(quote("plain/path"), "'plain/path'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn wrapper_files_are_private_and_idempotent() {
        let dir = wrapper_dir().expect("integration directory");
        assert!(dir.join(".zshenv").is_file());
        assert!(dir.join(".zshrc").is_file());
        assert!(dir.join("bashrc").is_file());
        assert_eq!(wrapper_dir().as_deref(), Some(dir.as_path()));
        let zsh = prepare("/bin/zsh", None).unwrap();
        assert_eq!(
            zsh.env.iter().find(|(key, _)| key == "ZDOTDIR").unwrap().1,
            dir.to_string_lossy()
        );
        let bash = prepare("/bin/bash", None).unwrap();
        assert_eq!(bash.args[0], "--rcfile");
    }
}
