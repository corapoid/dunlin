//! Filename-based syntax selection shared by local and remote workspaces.
//!
//! This catalog never reads file contents. Unknown text files can still be
//! displayed as plain text; a known syntax also admits a file to text preview.

use std::path::Path;

pub fn language(path: &str) -> Option<&'static str> {
    let name = Path::new(path).file_name()?.to_str()?.to_ascii_lowercase();
    if name == "dockerfile" || name.starts_with("dockerfile.") {
        return Some("dockerfile");
    }
    match name.as_str() {
        "makefile" | "gnumakefile" => return Some("make"),
        "cmakelists.txt" => return Some("cmake"),
        "cmakecache.txt" => return Some("cmakecache"),
        "cargo.lock" => return Some("toml"),
        "nginx.conf" => return Some("nginx"),
        "ansible.cfg" | "inventory" | "hosts" => return Some("ini"),
        ".ansible-lint" => return Some("yaml"),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" => return Some("bash"),
        _ => {}
    }
    if name == ".env" || name.starts_with(".env.") {
        return Some("bash");
    }
    let extension = Path::new(&name).extension()?.to_str()?;
    if matches!(extension, "j2" | "jinja" | "jinja2") {
        let stem = name.strip_suffix(extension)?.trim_end_matches('.');
        return Some(
            match Path::new(stem).extension().and_then(|ext| ext.to_str()) {
                Some("yaml" | "yml") => "yaml-jinja",
                Some("html" | "htm" | "xhtml" | "xml") => "html-jinja",
                _ => "jinja2",
            },
        );
    }
    Some(match extension {
        "rs" => "rust",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "md" | "markdown" => "markdown",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "css" => "css",
        "scss" => "scss",
        "sass" => "sass",
        "html" | "htm" => "html",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "sql" => "sql",
        "swift" => "swift",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "rb" => "ruby",
        "php" => "php",
        "xml" => "xml",
        "ini" | "cfg" | "conf" => "ini",
        "svelte" => "svelte",
        "vue" => "vue",
        "lua" => "lua",
        "ex" | "exs" => "elixir",
        "r" => "r",
        "pl" => "perl",
        "ps1" | "psm1" | "psd1" => "powershell",
        "bat" | "cmd" => "bat",
        "diff" | "patch" => "diff",
        "zig" | "zon" => "zig",
        "dart" => "dart",
        "cs" | "csx" => "csharp",
        "tf" | "tfvars" | "hcl" => "terraform",
        "graphql" | "graphqls" | "gql" => "graphql",
        "proto" => "proto",
        "nix" => "nix",
        "cmake" => "cmake",
        "dockerfile" => "dockerfile",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::language;
    use crate::preview::{PreviewKind, kind};

    #[test]
    fn tool_files_have_the_same_syntax_and_admission_on_every_backend() {
        for (path, expected) in [
            ("src/main.ZIG", "zig"),
            ("build.zig.zon", "zig"),
            ("lib/main.dart", "dart"),
            ("src/App.cs", "csharp"),
            ("script.csx", "csharp"),
            ("components/App.jsx", "jsx"),
            ("scripts/install.PS1", "powershell"),
            ("scripts/module.psm1", "powershell"),
            ("scripts/module.psd1", "powershell"),
            ("main.tf", "terraform"),
            ("terraform.tfvars", "terraform"),
            (".terraform.lock.hcl", "terraform"),
            ("main.tf.json", "json"),
            ("vars.tfvars.json", "json"),
            ("roles/web/tasks/main.yml", "yaml"),
            ("group_vars/all.yaml", "yaml"),
            ("templates/config.yml.j2", "yaml-jinja"),
            ("templates/config.YAML.JINJA2", "yaml-jinja"),
            ("templates/page.html.j2", "html-jinja"),
            ("templates/nginx.conf.j2", "jinja2"),
            ("template.jinja", "jinja2"),
            ("ansible.cfg", "ini"),
            ("inventory", "ini"),
            ("hosts", "ini"),
            (".ansible-lint", "yaml"),
            ("api/schema.graphql", "graphql"),
            ("api/schema.graphqls", "graphql"),
            ("query.gql", "graphql"),
            ("api/message.proto", "proto"),
            ("flake.nix", "nix"),
            ("CMakeLists.txt", "cmake"),
            ("build/CMakeCache.txt", "cmakecache"),
            ("cmake/FindLib.cmake", "cmake"),
            ("Dockerfile.dev", "dockerfile"),
            ("build.dockerfile", "dockerfile"),
            ("compose.yaml", "yaml"),
            (".github/workflows/ci.yml", "yaml"),
            (".gitlab-ci.yml", "yaml"),
            ("nginx.conf", "nginx"),
            ("other.conf", "ini"),
            ("GNUmakefile", "make"),
            (".zshrc", "bash"),
            (".env.custom", "bash"),
            ("script.cmd", "bat"),
        ] {
            assert_eq!(language(path), Some(expected), "{path}");
            assert_eq!(kind(path), PreviewKind::Text, "{path}");
        }
    }

    #[test]
    fn unknown_files_do_not_bypass_the_preview_allowlist() {
        for path in [
            "file.unknown",
            "archive.zip",
            "app.exe",
            "Dockerfileish",
            "",
        ] {
            assert_eq!(language(path), None, "{path}");
            assert_eq!(kind(path), PreviewKind::Unsupported, "{path}");
        }
        assert_eq!(kind("image.svg"), PreviewKind::Svg);
        assert_eq!(kind("README.txt"), PreviewKind::Text);
    }
}
