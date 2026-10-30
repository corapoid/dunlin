//! Bounded, read-only discovery. No recursive traversal, URL requests or config execution.
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_DOCUMENT: usize = 128 * 1024;
const MAX_CANDIDATES: usize = 512;
const MOUNTS: &[&str] = &["", "web", "frontend", "apps/web"];

pub(super) fn read_local(root: &Path, relative: &Path, limit: usize) -> Option<Vec<u8>> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    if relative.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some(".git" | "node_modules" | "target" | "dist" | "build")
        )
    }) {
        return None;
    }
    let path = fs::canonicalize(root.join(relative)).ok()?;
    if !path.starts_with(root) {
        return None;
    }
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn normalize(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            Component::ParentDir if result.pop() => {}
            _ => return None,
        }
    }
    Some(result)
}

fn local_url(url: &str) -> Option<String> {
    let url = url
        .trim()
        .strip_prefix("%PUBLIC_URL%")
        .unwrap_or(url.trim());
    let url = url.split(['?', '#']).next()?.replace("&amp;", "&");
    let mut decoded = Vec::new();
    let mut bytes = url.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.is_empty()
        || decoded.starts_with("//")
        || decoded.contains([':', '\\', '\0', '{', '}', '<', '>'])
    {
        return None;
    }
    Some(decoded)
}

fn references(owner: &Path, mount: &Path, url: &str) -> Vec<PathBuf> {
    let Some(url) = local_url(url) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if !url.starts_with('/')
        && let Some(path) = normalize(&owner.parent().unwrap_or(mount).join(&url))
    {
        paths.push(path);
    }
    for directory in ["public", "static", ""] {
        if let Some(path) = normalize(&mount.join(directory).join(url.trim_start_matches('/'))) {
            paths.push(path);
        }
    }
    paths
}

// Only literal tag/attribute values are inspected. Comments and script/style
// bodies are skipped; templates and JavaScript configuration are never evaluated.
fn links(html: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut index = 0;
    while index < bytes.len() && result.len() < 64 {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if lower[index..].starts_with("<!--") {
            index = lower[index + 4..]
                .find("-->")
                .map(|n| index + 4 + n + 3)
                .unwrap_or(bytes.len());
            continue;
        }
        index += 1;
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphanumeric() {
            index += 1;
        }
        let tag = &lower[start..index];
        if tag == "script" || tag == "style" {
            let closing = format!("</{tag}");
            index = lower[index..]
                .find(&closing)
                .map(|n| index + n + closing.len())
                .unwrap_or(bytes.len());
            continue;
        }
        let mut rel = None;
        let mut href = None;
        while index < bytes.len() && bytes[index] != b'>' {
            if bytes[index].is_ascii_whitespace() || bytes[index] == b'/' {
                index += 1;
                continue;
            }
            let start = index;
            while index < bytes.len()
                && !bytes[index].is_ascii_whitespace()
                && !matches!(bytes[index], b'=' | b'>')
            {
                index += 1;
            }
            let attribute = &lower[start..index];
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if bytes.get(index) != Some(&b'=') {
                continue;
            }
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let quote = bytes
                .get(index)
                .copied()
                .filter(|b| matches!(b, b'\'' | b'"'));
            if quote.is_some() {
                index += 1;
            }
            let start = index;
            while index < bytes.len()
                && match quote {
                    Some(quote) => bytes[index] != quote,
                    None => !bytes[index].is_ascii_whitespace() && bytes[index] != b'>',
                }
            {
                index += 1;
            }
            let value = &html[start..index];
            if quote.is_some() && index < bytes.len() {
                index += 1;
            }
            if tag == "link" {
                match attribute {
                    "rel" => rel = Some(value.to_ascii_lowercase()),
                    "href" => href = Some(value.to_string()),
                    _ => {}
                }
            }
        }
        if let (Some(rel), Some(href)) = (rel, href) {
            result.push((rel, href));
        }
    }
    result
}

pub(super) fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |path: PathBuf| {
        if result.len() < MAX_CANDIDATES && seen.insert(path.clone()) {
            result.push(path);
        }
    };
    let mut manifests = Vec::new();
    for mount in MOUNTS {
        let mount = Path::new(mount);
        for document in [
            "index.html",
            "public/index.html",
            "src/index.html",
            "src/app.html",
            "app.html",
        ] {
            let owner = mount.join(document);
            let Some(bytes) = read_local(root, &owner, MAX_DOCUMENT) else {
                continue;
            };
            let Ok(html) = std::str::from_utf8(&bytes) else {
                continue;
            };
            for (rel, href) in links(html) {
                if rel.split_ascii_whitespace().any(|part| {
                    matches!(
                        part,
                        "icon" | "apple-touch-icon" | "apple-touch-icon-precomposed" | "mask-icon"
                    )
                }) {
                    for path in references(&owner, mount, &href) {
                        push(path);
                    }
                } else if rel.split_ascii_whitespace().any(|part| part == "manifest") {
                    for path in references(&owner, mount, &href) {
                        manifests.push((mount.to_path_buf(), path));
                    }
                }
            }
        }
        for directory in ["", "public", "static"] {
            for name in ["manifest.json", "site.webmanifest", "manifest.webmanifest"] {
                manifests.push((mount.to_path_buf(), mount.join(directory).join(name)));
            }
        }
    }
    let mut read_manifests = HashSet::new();
    for (mount, owner) in manifests.into_iter().take(64) {
        if !read_manifests.insert(owner.clone()) {
            continue;
        }
        let Some(bytes) = read_local(root, &owner, MAX_DOCUMENT) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if let Some(icons) = value["icons"].as_array() {
            for icon in icons.iter().take(16) {
                if let Some(src) = icon["src"].as_str() {
                    for path in references(&owner, &mount, src) {
                        push(path);
                    }
                }
            }
        }
    }
    // Framework file-based icons, then conventional favicons, then logos.
    // Explicit HTML/manifest declarations above always take priority.
    for mount in MOUNTS {
        for directory in ["app", "src/app"] {
            for name in ["icon.svg", "icon.png", "favicon.ico", "apple-icon.png"] {
                push(Path::new(mount).join(directory).join(name));
            }
        }
    }
    for name in [
        "favicon.svg",
        "favicon.ico",
        "favicon.png",
        "apple-touch-icon.png",
        "icon.svg",
        "icon.png",
        "logo.svg",
        "logo.png",
        "logo.jpg",
    ] {
        for mount in MOUNTS {
            for directory in [
                "",
                "public",
                "static",
                "assets",
                "src/assets",
                "public/assets",
                "static/assets",
                "src",
            ] {
                push(Path::new(mount).join(directory).join(name));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{links, local_url, references};
    use std::path::{Path, PathBuf};
    #[test]
    fn html_recognizes_literal_link_attributes_without_executing_templates_or_scripts() {
        let html = r#"<!-- <link rel=icon href=wrong.png> -->
        <script>const example = '<link rel=icon href=wrong.png>';</script>
        <LINK HREF="/brand/my%20icon.svg?v=2" type="image/svg+xml" REL='shortcut icon'>
        <link rel=manifest href=/site.webmanifest>"#;
        assert_eq!(
            links(html),
            [
                ("shortcut icon".into(), "/brand/my%20icon.svg?v=2".into()),
                ("manifest".into(), "/site.webmanifest".into())
            ]
        );
    }
    #[test]
    fn reference_resolution_is_local_and_constrained_to_the_project() {
        let owner = Path::new("public/index.html");
        assert!(references(owner, Path::new(""), "https://example.com/icon.png").is_empty());
        for url in [
            "//host/icon.png",
            "data:image/png;base64,abc",
            "file:///secret",
            "%2f%2fhost/icon.png",
            "%2e%2e/%2e%2e/secret.png",
            "\\external\\icon.png",
            "{{icon}}",
        ] {
            assert!(references(owner, Path::new(""), url).is_empty(), "{url}");
        }
        let paths = references(owner, Path::new(""), "/brand/my%20icon.svg?v=2");
        assert_eq!(paths[0], PathBuf::from("public/brand/my icon.svg"));
        assert_eq!(
            local_url("%PUBLIC_URL%/favicon.ico").as_deref(),
            Some("/favicon.ico")
        );
        assert!(
            references(
                Path::new("web/public/manifest.json"),
                Path::new("web"),
                "icons/app.png"
            )
            .contains(&PathBuf::from("web/public/icons/app.png"))
        );
    }
    #[test]
    fn malformed_or_unicode_html_is_bounded_and_does_not_panic() {
        for html in [
            "<",
            "<link",
            "<link href='unterminated",
            "ąę🪶 <link rel=icon href='żółw.svg'>",
            "<link disabled=",
            "<link = >",
        ] {
            let _ = links(html);
        }
    }
}
