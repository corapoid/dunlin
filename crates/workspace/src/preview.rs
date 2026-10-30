//! Preview allowlist and bounded reads. Never sniff or decode exotic files.
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewKind {
    Text,
    Svg,
    Png,
    Jpeg,
    Pdf,
    Unsupported,
}
pub fn kind(path: &str) -> PreviewKind {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = Path::new(&name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match ext {
        "svg" => PreviewKind::Svg,
        "png" => PreviewKind::Png,
        "jpg" | "jpeg" => PreviewKind::Jpeg,
        "pdf" => PreviewKind::Pdf,
        "rs" | "md" | "markdown" | "txt" | "toml" | "json" | "jsonc" | "yaml" | "yml" | "js"
        | "mjs" | "cjs" | "ts" | "mts" | "cts" | "tsx" | "jsx" | "py" | "go" | "c" | "h" | "cc"
        | "cpp" | "hpp" | "css" | "scss" | "sass" | "html" | "htm" | "xml" | "sh" | "bash"
        | "zsh" | "sql" | "swift" | "java" | "kt" | "kts" | "rb" | "php" | "ini" | "cfg"
        | "conf" | "env" | "gitignore" | "gitattributes" | "editorconfig" | "lock" | "csv"
        | "tsv" | "log" | "svelte" | "vue" | "dockerfile" | "r" | "ex" | "exs" | "lua" | "pl"
        | "ps1" | "bat" | "cmd" | "diff" | "patch" => PreviewKind::Text,
        _ if matches!(
            name.as_str(),
            "dockerfile"
                | "makefile"
                | "license"
                | "licence"
                | "readme"
                | ".env"
                | ".gitignore"
                | ".gitattributes"
                | ".editorconfig"
                | ".npmrc"
                | ".yarnrc"
                | ".nvmrc"
                | ".bashrc"
                | ".zshrc"
                | ".prettierrc"
                | ".eslintrc"
        ) || (name.starts_with(".env.")
            && matches!(
                ext,
                "local"
                    | "production"
                    | "development"
                    | "testing"
                    | "test"
                    | "example"
                    | "sample"
                    | "staging"
            )) =>
        {
            PreviewKind::Text
        }
        _ => PreviewKind::Unsupported,
    }
}
pub const MAX_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;
pub fn read(root: &str, relative: &str) -> Result<Vec<u8>, String> {
    if matches!(kind(relative), PreviewKind::Text | PreviewKind::Unsupported) {
        return Err("Preview is not supported for this file type".into());
    }
    let path = crate::fs::resolve(root, relative)?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_PREVIEW_BYTES {
        return Err("Preview requires a regular file no larger than 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err("File grew beyond the preview size limit".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_files_are_rejected_before_reading_or_sniffing() {
        for name in ["a.zip", "a.avif", "a.mp4", "a.gif", "a.exe", "a.weird", "a"] {
            assert_eq!(kind(name), PreviewKind::Unsupported);
            assert!(
                read("/does-not-exist", name)
                    .unwrap_err()
                    .contains("not supported")
            );
        }
        for name in ["a.SVG", "a.JPG", "a.png", "a.pdf"] {
            assert!(!matches!(
                kind(name),
                PreviewKind::Text | PreviewKind::Unsupported
            ));
        }
        assert_eq!(kind(".env.local"), PreviewKind::Text);
    }
}
