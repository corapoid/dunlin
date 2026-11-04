use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_OUTPUT_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct CodexCli {
    pub program: PathBuf,
}

#[derive(Clone, Debug)]
pub struct CliOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Resolve `codex-security` or its `cs` alias from an absolute PATH entry.
/// Dunlin never runs `npx` or a repository-local binary implicitly.
pub fn discover() -> Option<CodexCli> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        for name in ["codex-security", "cs"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(CodexCli { program: candidate });
            }
        }
    }
    None
}

pub fn scan_args(repository: &Path, output_dir: &Path, mock: bool) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("scan"),
        repository.as_os_str().to_os_string(),
        OsString::from("--output-dir"),
        output_dir.as_os_str().to_os_string(),
        OsString::from("--json"),
    ];
    if mock {
        args.push(OsString::from("--mock"));
    }
    args
}

pub fn patch_args(occurrence_id: &str) -> Vec<OsString> {
    vec![
        OsString::from("patch"),
        OsString::from(occurrence_id),
        OsString::from("--json"),
    ]
}

pub fn verify_args(occurrence_id: &str) -> Vec<OsString> {
    vec![
        OsString::from("verify-fix"),
        OsString::from(occurrence_id),
        OsString::from("--json"),
    ]
}

fn clip(value: &[u8]) -> String {
    let value = &value[..value.len().min(MAX_OUTPUT_BYTES)];
    String::from_utf8_lossy(value).to_string()
}

/// Run an explicit argv without a shell. The caller must show a confirmation
/// before invoking a command that can modify a repository.
pub fn run(cli: &CodexCli, args: &[OsString], cwd: &Path) -> Result<CliOutput, String> {
    let output = Command::new(&cli.program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("Could not start {}: {error}", cli.program.display()))?;
    Ok(CliOutput {
        success: output.status.success(),
        exit_code: output.status.code(),
        stdout: clip(&output.stdout),
        stderr: clip(&output.stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_args_are_explicit_and_never_use_a_shell() {
        let args = scan_args(Path::new("/tmp/repo"), Path::new("/tmp/run/codex"), false);
        assert_eq!(args[0], "scan");
        assert_eq!(args[1], "/tmp/repo");
        assert_eq!(args[2], "--output-dir");
        assert_eq!(args[3], "/tmp/run/codex");
        assert!(!args.iter().any(|arg| arg.to_string_lossy().contains('|')));
    }

    #[test]
    fn patch_and_verify_target_the_occurrence_id() {
        assert_eq!(patch_args("occ_1"), vec!["patch", "occ_1", "--json"]);
        assert_eq!(verify_args("occ_1"), vec!["verify-fix", "occ_1", "--json"]);
        let mock = scan_args(Path::new("/tmp/repo"), Path::new("/tmp/out"), true);
        assert!(mock.iter().any(|arg| arg == "--mock"));
    }

    #[test]
    fn output_is_bounded() {
        assert!(clip(&vec![b'x'; MAX_OUTPUT_BYTES + 10]).len() <= MAX_OUTPUT_BYTES);
    }
}
