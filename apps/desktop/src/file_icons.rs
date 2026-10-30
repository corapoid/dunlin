//! Local, filename-only Explorer icons. No file contents or network are needed.
use dunlin_ui::{Hsla, Palette, assets::IconName};

pub(crate) fn appearance(name: &str, p: &Palette) -> (IconName, Hsla) {
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if lower.starts_with(".git") || lower == ".gitmodules" {
        return (IconName::GitBranch, p.red);
    }
    if lower == ".env" || lower.starts_with(".env.") || matches!(extension, "pem" | "key" | "crt") {
        return (IconName::FileKey, p.amber);
    }
    if lower == "dockerfile" || lower.starts_with("dockerfile.") || lower == ".dockerignore" {
        return (IconName::Container, p.blue);
    }
    if matches!(
        lower.as_str(),
        "package.json" | "cargo.toml" | "pyproject.toml" | "go.mod" | "gemfile" | "composer.json"
    ) {
        return (IconName::Package, p.green);
    }
    if matches!(extension, "lock")
        || matches!(
            lower.as_str(),
            "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock"
        )
    {
        return (IconName::FileLock, p.muted_fg);
    }
    if matches!(
        lower.as_str(),
        "makefile" | "justfile" | ".bashrc" | ".zshrc"
    ) {
        return (IconName::FileTerminal, p.green);
    }
    match extension {
        "tsx" | "jsx" => (IconName::Atom, p.blue),
        "ts" | "mts" | "cts" => (IconName::FileCode, p.blue),
        "js" | "mjs" | "cjs" => (IconName::FileCode, p.amber),
        "rs" => (IconName::FileCode, p.amber),
        "py" | "pyi" | "pyw" => (IconName::FileCode, p.green),
        "go" => (IconName::FileCode, p.teal),
        "html" | "htm" | "xml" | "vue" | "svelte" | "astro" => (IconName::CodeXml, p.red),
        "css" | "scss" | "sass" | "less" => (IconName::Brush, p.violet),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "rb" | "php"
        | "lua" | "ex" | "exs" | "zig" => (IconName::FileCode, p.violet),
        "json" | "jsonc" | "json5" => (IconName::FileBraces, p.amber),
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "config" => {
            (IconName::FileCog, p.violet)
        }
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => (IconName::FileTerminal, p.green),
        "md" | "mdx" | "markdown" | "rst" => (IconName::FileText, p.blue),
        "txt" | "log" => (IconName::FileText, p.muted_fg),
        "pdf" => (IconName::FileText, p.red),
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "ico" | "bmp" | "tiff" => {
            (IconName::FileImage, p.violet)
        }
        "sql" | "db" | "sqlite" | "sqlite3" => (IconName::Database, p.teal),
        "csv" | "tsv" | "xls" | "xlsx" => (IconName::FileSpreadsheet, p.green),
        "zip" | "gz" | "tar" | "bz2" | "xz" | "7z" | "rar" => (IconName::FileArchive, p.amber),
        "woff" | "woff2" | "ttf" | "otf" => (IconName::FileType, p.amber),
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => (IconName::FileMusic, p.violet),
        "mp4" | "mov" | "webm" | "mkv" => (IconName::FileVideoCamera, p.violet),
        _ => (IconName::File, p.muted_fg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_web_source_config_media_and_special_filenames() {
        use dunlin_ui::AssetSource;
        let p = dunlin_ui::theme::dark();
        for (name, expected) in [
            ("index.ts", IconName::FileCode),
            ("App.TSX", IconName::Atom),
            ("app.vue", IconName::CodeXml),
            ("style.scss", IconName::Brush),
            ("script.py", IconName::FileCode),
            ("Cargo.toml", IconName::Package),
            ("package.json", IconName::Package),
            ("pnpm-lock.yaml", IconName::FileLock),
            (".env.local", IconName::FileKey),
            (".gitignore", IconName::GitBranch),
            ("Dockerfile.dev", IconName::Container),
            ("Makefile", IconName::FileTerminal),
            ("logo.WEBP", IconName::FileImage),
            ("data.sqlite3", IconName::Database),
            ("README.md", IconName::FileText),
            ("archive.tar.gz", IconName::FileArchive),
            ("unknown.xyz", IconName::File),
            ("LICENSE", IconName::File),
        ] {
            assert_eq!(appearance(name, &p).0, expected, "{name}");
            assert!(
                dunlin_ui::assets::AllAssets
                    .load(expected.path().as_ref())
                    .unwrap()
                    .is_some(),
                "Missing bundled icon for {name}"
            );
        }
    }
}
