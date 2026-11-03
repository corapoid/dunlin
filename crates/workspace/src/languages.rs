//! Bounded, extension-based language statistics for the project list.
//!
//! This is intentionally not a full Linguist implementation: it never reads
//! file contents, skips generated/vendored directories and caps the scan so a
//! huge checkout cannot block the workspace summaries.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const MAX_FILES: usize = 50_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageStat {
    pub name: &'static str,
    pub color: u32,
    pub bytes: u64,
}

/// Return languages sorted from most to least bytes.
pub fn detect(root: &Path) -> Vec<LanguageStat> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut totals: BTreeMap<&'static str, (u32, u64)> = BTreeMap::new();
    let mut files = 0usize;
    let mut bytes = 0u64;
    walk(root, 0, &mut totals, &mut files, &mut bytes);
    let mut stats = totals
        .into_iter()
        .map(|(name, (color, bytes))| LanguageStat { name, color, bytes })
        .collect::<Vec<_>>();
    stats.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(b.name)));
    stats
}

fn walk(
    dir: &Path,
    depth: usize,
    totals: &mut BTreeMap<&'static str, (u32, u64)>,
    files: &mut usize,
    bytes: &mut u64,
) {
    if depth > MAX_DEPTH || *files >= MAX_FILES || *bytes >= MAX_BYTES {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if *files >= MAX_FILES || *bytes >= MAX_BYTES {
            break;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_skipped(name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk(&path, depth + 1, totals, files, bytes);
            continue;
        }
        if !file_type.is_file() || name.contains(".min.") {
            continue;
        }
        let Some((language, color)) = language_for(&path) else {
            continue;
        };
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(1).max(1);
        let total = totals.entry(language).or_insert((color, 0));
        total.1 = total.1.saturating_add(size);
        *files += 1;
        *bytes = bytes.saturating_add(size);
    }
}

fn is_skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".next"
                | ".cache"
                | ".turbo"
                | ".venv"
                | "venv"
                | "__pycache__"
                | "vendor"
                | "Pods"
                | "DerivedData"
        )
}

fn language_for(path: &Path) -> Option<(&'static str, u32)> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    if lower == "dockerfile" {
        return Some(("Dockerfile", 0x384d54));
    }
    if matches!(lower.as_str(), "makefile" | "gnumakefile") {
        return Some(("Makefile", 0x427819));
    }
    if lower == "cmakelists.txt" {
        return Some(("CMake", 0xda3434));
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "rs" => Some(("Rust", 0xdea584)),
        "ts" | "tsx" | "mts" | "cts" => Some(("TypeScript", 0x3178c6)),
        "js" | "jsx" | "mjs" | "cjs" => Some(("JavaScript", 0xf1e05a)),
        "py" | "pyi" => Some(("Python", 0x3572a5)),
        "go" => Some(("Go", 0x00add8)),
        "swift" => Some(("Swift", 0xf05138)),
        "c" => Some(("C", 0x555555)),
        "h" => Some(("C", 0x555555)),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => Some(("C++", 0xf34b7d)),
        "java" => Some(("Java", 0xb07219)),
        "kt" | "kts" => Some(("Kotlin", 0xa97bff)),
        "cs" => Some(("C#", 0x178600)),
        "fs" => Some(("F#", 0xb845fc)),
        "vb" => Some(("Visual Basic", 0x945db7)),
        "rb" => Some(("Ruby", 0x701516)),
        "php" => Some(("PHP", 0x4f5d95)),
        "pl" | "pm" => Some(("Perl", 0x0298c3)),
        "sh" | "bash" | "zsh" => Some(("Shell", 0x89e051)),
        "fish" => Some(("Fish", 0x4aae47)),
        "ps1" => Some(("PowerShell", 0x012456)),
        "bat" | "cmd" => Some(("Batchfile", 0xc1f12e)),
        "html" | "htm" => Some(("HTML", 0xe34c26)),
        "css" => Some(("CSS", 0x563d7c)),
        "scss" => Some(("SCSS", 0xc6538c)),
        "sass" => Some(("Sass", 0xa53b70)),
        "less" => Some(("Less", 0x1d365d)),
        "vue" => Some(("Vue", 0x41b883)),
        "svelte" => Some(("Svelte", 0xff3e00)),
        "astro" => Some(("Astro", 0xff5a03)),
        "dart" => Some(("Dart", 0x00b4ab)),
        "lua" => Some(("Lua", 0x000080)),
        "ex" | "exs" => Some(("Elixir", 0x6e4a7e)),
        "erl" | "hrl" => Some(("Erlang", 0xb83998)),
        "hs" | "lhs" => Some(("Haskell", 0x5e5086)),
        "clj" | "cljs" | "cljc" | "edn" => Some(("Clojure", 0xdb5855)),
        "scala" | "sc" => Some(("Scala", 0xc22d40)),
        "jl" => Some(("Julia", 0xa270ba)),
        "r" => Some(("R", 0x198ce7)),
        "m" => Some(("Objective-C", 0x438eff)),
        "mm" => Some(("Objective-C++", 0x6866fb)),
        "sol" => Some(("Solidity", 0xaa6746)),
        "zig" => Some(("Zig", 0xec915c)),
        "nim" => Some(("Nim", 0xffc200)),
        "s" | "asm" => Some(("Assembly", 0x6e4c13)),
        "sql" => Some(("SQL", 0xe38c00)),
        "ipynb" => Some(("Jupyter Notebook", 0xda5b0b)),
        "md" | "markdown" => Some(("Markdown", 0x083fa1)),
        "toml" | "yaml" | "yml" | "json" | "lock" | "xml" | "txt" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::detect;
    use std::fs;

    #[test]
    fn language_stats_ignore_generated_dirs_and_sort_by_bytes() {
        let root = std::env::temp_dir().join(format!("dunlin-languages-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("src/main.rs"), vec![b'r'; 120]).unwrap();
        fs::write(root.join("src/app.ts"), vec![b't'; 80]).unwrap();
        fs::write(root.join("README.md"), vec![b'#'; 10]).unwrap();
        fs::write(root.join("target/generated.rs"), vec![b'x'; 10_000]).unwrap();
        fs::write(root.join("node_modules/dep.js"), vec![b'j'; 10_000]).unwrap();

        let stats = detect(&root);
        assert_eq!(stats[0].name, "Rust");
        assert_eq!(stats[0].bytes, 120);
        assert_eq!(stats[1].name, "TypeScript");
        assert_eq!(stats[2].name, "Markdown");
        assert!(!stats.iter().any(|stat| stat.name == "JavaScript"));
        fs::remove_dir_all(root).unwrap();
    }
}
