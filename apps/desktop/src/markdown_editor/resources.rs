//! Source-backed resource properties and project-scoped explicit resource reads.
use super::{Mode, document::Document, editable};
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{
        Disableable, Sizable,
        button::ButtonVariants,
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};
use markdown::{ParseOptions, mdast::Node};
use std::{ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Target {
    pub raw: Range<usize>,
    properties: Range<usize>,
    pub url: String,
    title: String,
    pub image: bool,
    reference: bool,
}
pub(super) struct Draft {
    target: Target,
    snapshot: String,
    project: String,
    root: String,
    pub(super) address: Entity<InputState>,
    pub(super) title: Entity<InputState>,
    pub(super) preview: Option<crate::file_preview::Preview>,
    pending: bool,
    generation: uuid::Uuid,
    preview_address: Option<String>,
}

fn ast(source: &str) -> Result<Node, String> {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    markdown::to_mdast(source, &options).map_err(|e| e.to_string())
}
fn nodes<'a>(node: &'a Node, result: &mut Vec<&'a Node>, depth: usize) {
    if depth > 48 || result.len() >= 10_000 {
        return;
    }
    result.push(node);
    if let Some(children) = node.children() {
        for child in children {
            nodes(child, result, depth + 1);
        }
    }
}
fn targets(source: &str) -> Result<Vec<Target>, String> {
    let ast = ast(source)?;
    let mut all = Vec::new();
    nodes(&ast, &mut all, 0);
    let mut result = Vec::new();
    for node in &all {
        let Some(p) = node.position() else {
            continue;
        };
        let raw = p.start.offset..p.end.offset;
        let (url, title, image, reference, properties) = match node {
            Node::Link(n) => (
                n.url.clone(),
                n.title.clone().unwrap_or_default(),
                false,
                false,
                raw.clone(),
            ),
            Node::Image(n) => (
                n.url.clone(),
                n.title.clone().unwrap_or_default(),
                true,
                false,
                raw.clone(),
            ),
            Node::LinkReference(n) => {
                let Some(Node::Definition(d)) = all
                    .iter()
                    .find(|x| matches!(x,Node::Definition(d) if d.identifier==n.identifier))
                    .copied()
                else {
                    continue;
                };
                let p = d.position.as_ref().ok_or("Missing reference position")?;
                (
                    d.url.clone(),
                    d.title.clone().unwrap_or_default(),
                    false,
                    true,
                    p.start.offset..p.end.offset,
                )
            }
            Node::ImageReference(n) => {
                let Some(Node::Definition(d)) = all
                    .iter()
                    .find(|x| matches!(x,Node::Definition(d) if d.identifier==n.identifier))
                    .copied()
                else {
                    continue;
                };
                let p = d.position.as_ref().ok_or("Missing reference position")?;
                (
                    d.url.clone(),
                    d.title.clone().unwrap_or_default(),
                    true,
                    true,
                    p.start.offset..p.end.offset,
                )
            }
            _ => continue,
        };
        result.push(Target {
            raw,
            properties,
            url,
            title,
            image,
            reference,
        });
    }
    Ok(result)
}

#[derive(Debug, PartialEq, Eq)]
enum Address {
    External(String),
    Project(String),
}
fn scope_current(this: &DunlinApp, tab: &crate::workbench::Tab) -> bool {
    use crate::remote::WorkspaceBackend;
    let project = this.project();
    if tab.project != project.id || tab.root != project.path || !project.trusted {
        return false;
    }
    match (&tab.backend, &project.remote) {
        (WorkspaceBackend::Local { root }, None) => root == &project.path,
        (WorkspaceBackend::Remote(remote), Some(current)) => {
            remote.project.host_id == current.host_id
                && remote.project.project_id == current.project_id
                && remote.project.path == current.path
                && this.remote_hosts.iter().any(|h| {
                    h.id == remote.host.id
                        && h.url == remote.host.url
                        && h.device_id == remote.host.device_id
                })
        }
        _ => false,
    }
}
fn resolve(root: &str, document: &str, value: &str) -> Result<Address, String> {
    if value.is_empty() || value.len() > 8192 || value.chars().any(|c| c.is_control() || c == '\\')
    {
        return Err("Invalid resource address".into());
    }
    let first = value.split(['/', '?', '#']).next().unwrap_or("");
    if first.contains(':') {
        let url = url::Url::parse(value).map_err(|_| "Invalid URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Only HTTP(S) links without credentials may be opened".into());
        }
        return Ok(Address::External(url.to_string()));
    }
    if value.starts_with('/') {
        return Err(
            "Resources must be relative to this document and stay inside the project".into(),
        );
    }
    let path = value.split(['?', '#']).next().unwrap_or("");
    let mut decoded = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = path.get(i + 1..i + 3).ok_or("Invalid percent encoding")?;
            let byte = u8::from_str_radix(pair, 16).map_err(|_| "Invalid percent encoding")?;
            if matches!(byte, b'/' | b'\\') {
                return Err("Encoded path separators are not permitted".into());
            }
            decoded.push(byte);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| "Invalid UTF-8 path")?;
    if decoded
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '%' | ':' | '?' | '#'))
        || decoded.starts_with('/')
    {
        return Err("Unsafe encoded resource path".into());
    }
    let relative = document
        .strip_prefix(root.trim_end_matches('/'))
        .and_then(|s| s.strip_prefix('/'))
        .ok_or("Document is outside its project")?;
    let mut parts = relative.split('/').collect::<Vec<_>>();
    parts.pop();
    if decoded.is_empty() {
        return Ok(Address::Project(document.into()));
    }
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or("Resource escapes the project")?;
            }
            other => parts.push(other),
        }
    }
    Ok(Address::Project(format!(
        "{}/{}",
        root.trim_end_matches('/'),
        parts.join("/")
    )))
}
fn destination(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '\\' => "&#92;".to_string(),
            _ => c.to_string(),
        })
        .collect()
}
fn properties(
    source: &str,
    target: &Target,
    url: &str,
    title: &str,
) -> Result<(Range<usize>, String, Document), String> {
    if url.len() > 8192
        || title.len() > 4096
        || url.chars().any(char::is_control)
        || title.chars().any(char::is_control)
    {
        return Err("Resource properties are too large or contain control characters".into());
    }
    let old = source
        .get(target.properties.clone())
        .ok_or("The resource changed")?;
    let expected_title = title;
    let title = title
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('\\', "&#92;");
    let suffix = if title.is_empty() {
        String::new()
    } else {
        format!(" \"{title}\"")
    };
    let value = if target.reference {
        let colon = old.find("]:").ok_or("Unsupported definition")? + 2;
        format!("{} <{}>{suffix}", &old[..colon], destination(url))
    } else {
        let mut depth = 0;
        let mut escaped = false;
        let mut close = None;
        for (at, c) in old.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '[' {
                depth += 1;
            }
            if c == ']' {
                depth -= 1;
                if depth == 0 {
                    close = Some(at + 1);
                    break;
                }
            }
        }
        let label = if let Some(close) = close {
            old[..close].to_string()
        } else {
            format!("[{}]", super::document::escaped(&target.url))
        };
        format!("{label}(<{}>{suffix})", destination(url))
    };
    let range = target.properties.clone();
    let mut next = source.to_string();
    next.replace_range(range.clone(), &value);
    let document = Document::parse(&next)?;
    let target_start = if range.end <= target.raw.start {
        target.raw.start - range.len() + value.len()
    } else {
        target.raw.start
    };
    let changed = targets(&next)?
        .into_iter()
        .find(|t| t.raw.start == target_start && t.image == target.image)
        .ok_or("The properties would change Markdown structure")?;
    if changed.url != url || changed.title != expected_title {
        return Err("The properties cannot be represented safely in Markdown".into());
    }
    Ok((range, value, document))
}

impl DunlinApp {
    pub(super) fn visual_resource_target(&self, cx: &App) -> Option<Target> {
        let tab = self
            .active_file_tab()
            .filter(|t| t.markdown.mode == Mode::Visual)?;
        let active = tab.markdown.visual.active.as_ref()?;
        if *tab.editor.read(cx).text() != active.snapshot.as_ref() {
            return None;
        }
        let caret = active.editor.read(cx).cursor();
        let raw = active.block.source_selection(caret..caret)?.start;
        targets(&active.snapshot)
            .ok()?
            .into_iter()
            .filter(|t| t.raw.start <= raw && raw <= t.raw.end)
            .min_by_key(|t| t.raw.len())
    }
    pub(super) fn open_visual_resource(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.visual_resource_target(cx) else {
            return;
        };
        let tab = self.active_file_tab().unwrap();
        let snapshot = tab.editor.read(cx).text().to_string();
        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Address"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Optional title"));
        address.update(cx, |s, cx| s.set_value(&target.url, window, cx));
        title.update(cx, |s, cx| s.set_value(&target.title, window, cx));
        let draft = Draft {
            target,
            snapshot,
            project: tab.project.clone(),
            root: tab.root.clone(),
            address,
            title,
            preview: None,
            pending: false,
            generation: uuid::Uuid::now_v7(),
            preview_address: None,
        };
        let id = tab.id.clone();
        self.workbench
            .tabs
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .markdown
            .resource = Some(draft);
        cx.notify();
    }
    pub(super) fn close_visual_resource(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_file_tab().map(|t| t.id.clone()) {
            self.workbench
                .tabs
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .markdown
                .resource = None;
        }
        cx.notify();
    }
    pub(super) fn save_visual_resource(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| editable(self, t) && t.markdown.mode == Mode::Visual)
        else {
            return;
        };
        let Some(d) = &tab.markdown.resource else {
            return;
        };
        if *tab.editor.read(cx).text() != d.snapshot {
            self.file_notice("The source changed. Reopen resource properties.", cx);
            return;
        }
        let url = d.address.read(cx).value().to_string();
        let title = d.title.read(cx).value().to_string();
        let result = resolve(&tab.root, &tab.inspector.path, &url)
            .and_then(|_| properties(&d.snapshot, &d.target, &url, &title));
        let (range, value, document) = match result {
            Ok(v) => v,
            Err(e) => {
                self.file_notice(e, cx);
                return;
            }
        };
        let id = tab.id.clone();
        let editor = tab.editor.clone();
        self.clear_visual_selection(cx);
        editor.update(cx, |s, cx| {
            s.set_selected_range(range, cx);
            s.replace(value, window, cx);
        });
        let tab = self.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.markdown.visual.document = Some(document);
        tab.markdown.resource = None;
        cx.notify();
    }
    pub(super) fn activate_visual_resource(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self
            .active_file_tab()
            .filter(|t| !t.loading && scope_current(self, t) && t.markdown.mode == Mode::Visual)
        else {
            return;
        };
        let Some(d) = &tab.markdown.resource else {
            return;
        };
        if *tab.editor.read(cx).text() != d.snapshot {
            self.file_notice("The source changed. Reopen resource properties.", cx);
            return;
        }
        let url = d.address.read(cx).value().to_string();
        let address = match resolve(&tab.root, &tab.inspector.path, &url) {
            Ok(v) => v,
            Err(e) => {
                self.file_notice(e, cx);
                return;
            }
        };
        let id = tab.id.clone();
        let snapshot = d.snapshot.clone();
        let project = d.project.clone();
        let root = d.root.clone();
        match address {
            Address::External(external) => {
                if d.target.image {
                    self.file_notice(
                        "External images are never fetched. Choose a project image.",
                        cx,
                    );
                    return;
                }
                let receiver = window.prompt(
                    PromptLevel::Warning,
                    "Open external link?",
                    Some(&external),
                    &["Cancel", "Open in browser"],
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    if receiver.await == Ok(1) {
                        let _ = this.update(cx, |this, cx| {
                            if this.active_file_tab().is_some_and(|t| {
                                scope_current(this, t)
                                    && t.id == id
                                    && t.project == project
                                    && t.root == root
                                    && t.markdown.mode == Mode::Visual
                                    && *t.editor.read(cx).text() == snapshot
                                    && t.markdown
                                        .resource
                                        .as_ref()
                                        .is_some_and(|d| d.address.read(cx).value() == url)
                            }) {
                                cx.open_url(&external);
                            }
                        });
                    }
                })
                .detach();
            }
            Address::Project(path) => {
                if !d.target.image {
                    self.open_workbench_file(&path, None, window, cx);
                    return;
                }
                let kind = dunlin_workspace::preview::kind(&path);
                if !matches!(
                    kind,
                    dunlin_workspace::preview::PreviewKind::Png
                        | dunlin_workspace::preview::PreviewKind::Jpeg
                        | dunlin_workspace::preview::PreviewKind::Svg
                ) {
                    self.file_notice(
                        "Image preview supports project PNG, JPEG and SVG files only.",
                        cx,
                    );
                    return;
                }
                let backend = tab.backend.clone();
                let generation = uuid::Uuid::now_v7();
                let draft = self
                    .workbench
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .markdown
                    .resource
                    .as_mut()
                    .unwrap();
                draft.generation = generation;
                draft.pending = true;
                draft.preview = None;
                draft.preview_address = Some(url.clone());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let bytes = backend.preview_bytes(&path)?;
                            crate::file_preview::decode(Arc::new(bytes), kind, 0)
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        let active = this.active_file_tab().is_some_and(|t| {
                            scope_current(this, t)
                                && t.id == id
                                && t.project == project
                                && t.markdown.mode == Mode::Visual
                        }) && this.project().id == project
                            && this.project().trusted;
                        let Some(tab) = this
                            .workbench
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == id && t.project == project && t.root == root)
                        else {
                            return;
                        };
                        let Some(d) = tab
                            .markdown
                            .resource
                            .as_mut()
                            .filter(|d| d.generation == generation)
                        else {
                            return;
                        };
                        d.pending = false;
                        if !active
                            || d.snapshot != snapshot
                            || d.address.read(cx).value() != url
                            || *tab.editor.read(cx).text() != snapshot
                        {
                            cx.notify();
                            return;
                        }
                        match result {
                            Ok(preview) => {
                                d.preview = Some(preview);
                                d.preview_address = Some(url);
                            }
                            Err(e) => this.file_notice(e, cx),
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            }
        }
    }
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Option<AnyElement> {
    let tab = this.active_file_tab()?;
    let d = tab.markdown.resource.as_ref()?;
    if tab.markdown.mode != Mode::Visual {
        return None;
    }
    let p = palette(cx);
    let stale = *tab.editor.read(cx).text() != d.snapshot || !scope_current(this, tab);
    let pending =
        d.pending && d.preview_address.as_deref() == Some(d.address.read(cx).value().as_ref());
    Some(
        div()
            .flex_shrink_0()
            .p(px(8.))
            .border_b_1()
            .border_color(p.border)
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(11.))
            .child(if d.target.image {
                "Image · description is editable in the document"
            } else if d.target.reference {
                "Reference link · changing properties updates all uses"
            } else {
                "Link properties"
            })
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(Input::new(&d.address).small())
                    .child(Input::new(&d.title).small()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        dunlin_ui::button("markdown-resource-apply")
                            .ghost()
                            .small()
                            .label("Apply")
                            .disabled(stale || !editable(this, tab))
                            .on_click(
                                cx.listener(|this, _, w, cx| this.save_visual_resource(w, cx)),
                            ),
                    )
                    .child(
                        dunlin_ui::button("markdown-resource-open")
                            .ghost()
                            .small()
                            .label(if pending {
                                "Loading…"
                            } else if d.target.image {
                                "Load project image"
                            } else {
                                "Open link…"
                            })
                            .disabled(stale || pending)
                            .on_click(
                                cx.listener(|this, _, w, cx| this.activate_visual_resource(w, cx)),
                            ),
                    )
                    .child(
                        dunlin_ui::button("markdown-resource-cancel")
                            .ghost()
                            .small()
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.close_visual_resource(cx))),
                    ),
            )
            .when(stale, |r| {
                r.child("Source changed — close and reopen properties.")
            })
            .when_some(
                d.preview.as_ref().filter(|_| {
                    !stale
                        && d.preview_address.as_deref() == Some(d.address.read(cx).value().as_ref())
                }),
                |r, preview| {
                    r.child(
                        img(preview.image.clone())
                            .h(px(160.))
                            .max_w_full()
                            .object_fit(ObjectFit::Contain),
                    )
                },
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Address, properties, resolve, targets};
    #[test]
    fn properties_preserve_labels_definitions_and_unrelated_source() {
        let source =
            "before\n\n[**label**](old \"title\") ![alt][pic]\n\n[pic]: image.png \"old\"\n\nend";
        let all = targets(source).unwrap();
        let (_, _, doc) = properties(
            source,
            &all[0],
            "https://example.com/new",
            "A \"title\" & more",
        )
        .unwrap();
        assert!(
            doc.source.contains(
                "[**label**](<https://example.com/new> \"A &quot;title&quot; &amp; more\")"
            )
        );
        assert!(doc.source.ends_with("\n\nend"));
        let (_, _, doc) = properties(source, &all[1], "assets/new.png", "new").unwrap();
        assert!(doc.source.contains("![alt][pic]"));
        assert!(doc.source.contains("[pic]: <assets/new.png> \"new\""));
    }
    #[test]
    fn project_resolution_never_interprets_remote_roots_locally() {
        assert_eq!(
            resolve(
                "/remote/project",
                "/remote/project/docs/a.md",
                "../images/a%20b.png"
            )
            .unwrap(),
            Address::Project("/remote/project/images/a b.png".into())
        );
        for url in [
            "../../escape.png",
            "%2fetc/passwd",
            "../%2e%2e/secret",
            "%252e%252e/a",
            "file:///etc/passwd",
            "data:image/png,x",
            "javascript:alert(1)",
            "https://u:p@example.com/x",
            "//host/image",
            "a%5cb.png",
            "a%2Fb.png",
            "a%00.png",
            "C:/a.png",
        ] {
            assert!(
                resolve("/project", "/project/docs/a.md", url).is_err(),
                "{url}"
            );
        }
        assert!(matches!(
            resolve("/project", "/project/a.md", "https://example.com/x").unwrap(),
            Address::External(_)
        ));
    }
    #[test]
    fn properties_handle_escaping_unicode_and_definitions_before_their_uses() {
        let source = "[pic]: old.png\n\n![żółw][pic]\n\n[link](old)\n";
        let all = targets(source).unwrap();
        let (_, _, doc) =
            properties(source, &all[0], "assets/żółw image.png", "&quot; \\").unwrap();
        assert!(doc.source.starts_with("[pic]: <assets/żółw image.png>"));
        let all = targets(&doc.source).unwrap();
        assert_eq!(all[0].title, "&quot; \\");
        let (_, _, doc) = properties(
            &doc.source,
            &all[1],
            "https://example.com/?a=&copy;&b=2",
            "",
        )
        .unwrap();
        assert_eq!(
            targets(&doc.source).unwrap()[1].url,
            "https://example.com/?a=&copy;&b=2"
        );
    }
    #[test]
    fn explicit_local_preview_uses_backend_confinement_and_bounds() {
        use crate::remote::WorkspaceBackend;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.png");
        std::fs::write(&outside, b"secret").unwrap();
        let root = root.to_string_lossy().into_owned();
        let backend = WorkspaceBackend::Local { root: root.clone() };
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, format!("{root}/escape.png")).unwrap();
            assert!(
                backend
                    .preview_bytes(&format!("{root}/escape.png"))
                    .is_err()
            );
        }
        assert!(backend.preview_bytes(&outside.to_string_lossy()).is_err());
        let huge = std::fs::File::create(format!("{root}/huge.png")).unwrap();
        huge.set_len(dunlin_workspace::preview::MAX_PREVIEW_BYTES + 1)
            .unwrap();
        assert!(backend.preview_bytes(&format!("{root}/huge.png")).is_err());
        std::fs::write(format!("{root}/bad.png"), b"not an image").unwrap();
        let bytes = backend.preview_bytes(&format!("{root}/bad.png")).unwrap();
        assert!(
            crate::file_preview::decode(
                std::sync::Arc::new(bytes),
                dunlin_workspace::preview::PreviewKind::Png,
                0
            )
            .is_err()
        );
    }
    #[test]
    fn explicit_remote_preview_is_authenticated_project_scoped_and_has_no_local_fallback() {
        use crate::remote::{RemoteHost, RemoteWorkspace, WorkspaceBackend};
        use base64::Engine;
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let body=serde_json::json!({"contents":base64::engine::general_purpose::STANDARD.encode(b"remote image bytes")}).to_string();
        let task = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    break;
                }
                assert!(bytes.len() < 16 * 1024);
            }
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let backend = WorkspaceBackend::Remote(Box::new(RemoteWorkspace {
            host: RemoteHost {
                id: "host".into(),
                name: "Mock".into(),
                url,
                user_name: "user".into(),
                user_role: "member".into(),
                device_id: "device".into(),
            },
            project: dunlin_domain::RemoteProject {
                host_id: "host".into(),
                project_id: "project-id".into(),
                path: "/server/project".into(),
            },
            token: "markdown-test-token".into(),
        }));
        let Address::Project(path) = resolve(
            "/server/project",
            "/server/project/docs/a.md",
            "../images/a%20b.png",
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(backend.preview_bytes(&path).unwrap(), b"remote image bytes");
        let request = task.join().unwrap();
        assert!(
            request.starts_with("GET /v1/fs/preview?project=project-id&path=images/a%20b.png ")
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer markdown-test-token")
        );
        assert!(backend.preview_bytes("/outside/local.png").is_err());
    }
}
