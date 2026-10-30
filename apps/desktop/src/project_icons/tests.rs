use super::{decode, detect, discovery, monogram};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("dunlin-project-icons-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn detect(&self, previous: Option<&super::Detected>) -> Option<super::Detected> {
        detect(self.0.to_str().unwrap(), previous)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn raster(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(width, height)
        .write_to(&mut output, format)
        .unwrap();
    output.into_inner()
}
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100"><rect width="400" height="100" fill="red"/></svg>"#;

#[test]
fn custom_icons_are_embedded_bounded_and_survive_source_file_removal() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let repo = Repo::new();
    repo.write("selected.svg", SVG);
    let image = decode(SVG.as_bytes(), "svg").unwrap();
    let saved = format!("png:{}", STANDARD.encode(image.bytes()));
    fs::remove_file(repo.0.join("selected.svg")).unwrap();
    let restored = super::custom_image(&saved).unwrap();
    assert_eq!(
        image::load_from_memory(image.bytes()).unwrap().to_rgba8(),
        image::load_from_memory(restored.bytes())
            .unwrap()
            .to_rgba8()
    );
    assert!(super::custom_image("png:invalid!").is_err());
    assert!(super::custom_image("no-format").is_err());
    let settings: dunlin_domain::ProjectSettings = serde_json::from_str("{}").unwrap();
    assert!(settings.custom_icon.is_none());
    let settings = dunlin_domain::ProjectSettings {
        custom_icon: Some(saved.clone()),
        ..Default::default()
    };
    let restored: dunlin_domain::ProjectSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(restored.custom_icon.as_deref(), Some(saved.as_str()));
}

#[test]
fn declared_favicon_wins_over_generic_assets_and_detection_is_read_only() {
    let repo = Repo::new();
    let html = r#"<link rel="icon" href="/brand/product.svg?v=1">"#;
    repo.write("index.html", html);
    repo.write("public/brand/product.svg", SVG);
    repo.write(
        "public/favicon.png",
        raster(image::ImageFormat::Png, 16, 16),
    );
    repo.write("logo.svg", SVG);
    let icon = repo.detect(None).unwrap();
    assert_eq!(icon.source, Path::new("public/brand/product.svg"));
    assert_eq!(fs::read_to_string(repo.0.join("index.html")).unwrap(), html);
    assert!(!repo.0.join("AGENTS.md").exists());
    assert_eq!(fs::read_dir(&repo.0).unwrap().count(), 3);
}

#[test]
fn manifest_icons_next_routes_and_logo_fallbacks_are_detected() {
    let repo = Repo::new();
    repo.write(
        "public/site.webmanifest",
        r#"{"icons":[{"src":"icons/product.png","sizes":"192x192"}]}"#,
    );
    repo.write(
        "public/icons/product.png",
        raster(image::ImageFormat::Png, 192, 192),
    );
    assert_eq!(
        repo.detect(None).unwrap().source,
        Path::new("public/icons/product.png")
    );
    let next = Repo::new();
    next.write("src/app/icon.svg", SVG);
    assert_eq!(
        next.detect(None).unwrap().source,
        Path::new("src/app/icon.svg")
    );
    let logo = Repo::new();
    logo.write("assets/logo.svg", SVG);
    assert_eq!(
        logo.detect(None).unwrap().source,
        Path::new("assets/logo.svg")
    );
    let web = Repo::new();
    web.write(
        "web/public/favicon.ico",
        raster(image::ImageFormat::Ico, 32, 32),
    );
    assert_eq!(
        web.detect(None).unwrap().source,
        Path::new("web/public/favicon.ico")
    );
}

#[test]
fn ico_png_jpeg_and_svg_are_small_contained_rasters() {
    for format in [
        image::ImageFormat::Ico,
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
    ] {
        let image = if format == image::ImageFormat::Jpeg {
            let mut output = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(64, 32)
                .write_to(&mut output, format)
                .unwrap();
            output.into_inner()
        } else {
            raster(format, 64, 32)
        };
        let icon = decode(&image, "ico").unwrap();
        let decoded = image::load_from_memory(icon.bytes()).unwrap();
        assert!(decoded.width() <= 128 && decoded.height() <= 128);
        assert_eq!(decoded.width(), decoded.height() * 2);
    }
    let icon = decode(SVG.as_bytes(), "svg").unwrap();
    let decoded = image::load_from_memory(icon.bytes()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (128, 32));
}

#[test]
fn cached_images_are_reused_and_updated_or_removed_when_assets_change() {
    let repo = Repo::new();
    repo.write("favicon.svg", SVG);
    let first = repo.detect(None).unwrap();
    let second = repo.detect(Some(&first)).unwrap();
    assert!(Arc::ptr_eq(&first.image, &second.image));
    repo.write("favicon.svg", SVG.replace("red", "blue"));
    let updated = repo.detect(Some(&first)).unwrap();
    assert!(!Arc::ptr_eq(&first.image, &updated.image));
    assert_ne!(first.signature, updated.signature);
    fs::remove_file(repo.0.join("favicon.svg")).unwrap();
    assert!(repo.detect(Some(&updated)).is_none());
}

#[test]
fn malformed_oversized_and_external_assets_do_not_prevent_a_safe_fallback() {
    let repo = Repo::new();
    repo.write(
        "index.html",
        r#"<link rel=icon href="https://host/private.png"><link rel=icon href="/invalid.svg">"#,
    );
    repo.write("public/invalid.svg", "not an SVG");
    repo.write("favicon.png", vec![0; super::MAX_ICON_BYTES + 1]);
    repo.write("logo.svg", SVG);
    assert_eq!(repo.detect(None).unwrap().source, Path::new("logo.svg"));
    assert!(discovery::read_local(&repo.0, Path::new("../outside"), 100).is_none());
    assert!(discovery::read_local(&repo.0, Path::new("/etc/hosts"), 100).is_none());
    assert!(decode(&vec![0; super::MAX_ICON_BYTES + 1], "png").is_err());
    assert!(decode(&raster(image::ImageFormat::Png, 2049, 1), "png").is_err());
    assert!(
        decode(
            br#"<!DOCTYPE svg [<!ENTITY external SYSTEM "file:///etc/passwd">]><svg/>"#,
            "svg"
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_assets_cannot_read_outside_the_repository() {
    let repo = Repo::new();
    let outside = Repo::new();
    outside.write("favicon.svg", SVG);
    std::os::unix::fs::symlink(outside.0.join("favicon.svg"), repo.0.join("favicon.svg")).unwrap();
    assert!(repo.detect(None).is_none());
    fs::remove_file(repo.0.join("favicon.svg")).unwrap();
    std::os::unix::fs::symlink(&outside.0, repo.0.join("public")).unwrap();
    assert!(repo.detect(None).is_none());
}

#[test]
fn monogram_fallback_preserves_the_sidebar_identity() {
    assert_eq!(monogram("Dunlin"), "D");
    assert_eq!(monogram("My Project"), "MP");
    assert_eq!(monogram("żółw app"), "ŻA");
    assert_eq!(monogram(""), "");
}

#[test]
fn cached_icons_and_background_results_are_scoped_to_the_current_trusted_local_root() {
    let mut project = dunlin_domain::Project {
        id: "project".into(),
        name: "Project".into(),
        path: "/project".into(),
        branch: None,
        tone: dunlin_domain::ProjectTone::Blue,
        pinned: false,
        trusted: true,
        running_sessions: 0,
        remote: None,
        group_id: None,
        settings: Default::default(),
    };
    assert!(super::matches_project(&project, "project", "/project"));
    assert!(!super::matches_project(&project, "other", "/project"));
    assert!(!super::matches_project(
        &project,
        "project",
        "/old-location"
    ));
    project.trusted = false;
    assert!(!super::matches_project(&project, "project", "/project"));
    project.trusted = true;
    project.remote = Some(Default::default());
    assert!(!super::matches_project(&project, "project", "/project"));
}
