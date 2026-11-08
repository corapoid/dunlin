//! Native spec workspace: intent, spec-kit artifacts and the project agent
//! contract. Files are canonical; this view only indexes, previews and records
//! explicit local decisions bound to an exact revision hash.

use crate::app::{DunlinApp, SettingsTab};
use crate::harness::ChatItem;
use crate::remote::WorkspaceBackend;
use crate::skills::{SPEC_KIT_SKILL, Scope, Target};
use dunlin_artifacts::{
    self as artifacts, Artifact, ContractAutonomy, IntentDraft, Kind, PhaseState, Snapshot,
    SpecPersistence, Status,
};
use dunlin_domain::Project;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::input::{Input, InputState};
use dunlin_ui::component::text::TextView;
use dunlin_ui::component::{Icon, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use dunlin_workspace::tree::FileTree;
use std::collections::HashSet;
use std::path::Path;

#[derive(Default)]
pub(crate) struct State {
    pub view: Option<View>,
    pub active: bool,
    pub decision_note: Option<Entity<InputState>>,
}

pub(crate) struct View {
    pub project: Project,
    pub snapshot: Snapshot,
    pub approvals: Vec<dunlin_storage::ArtifactApproval>,
    pub evidence: Vec<dunlin_storage::TaskEvidence>,
    pub selected: Option<String>,
    pub persistence: SpecPersistence,
    pub inbox: bool,
    pub trace: bool,
    pub expanded_requirements: HashSet<String>,
    pub expanded_tasks: HashSet<String>,
    pub loading: bool,
    pub error: Option<String>,
}

impl State {
    pub(crate) fn is_open_for(&self, project: &Project) -> bool {
        self.view
            .as_ref()
            .is_some_and(|view| owns(&view.project, project))
    }
    pub(crate) fn is_active_for(&self, project: &Project) -> bool {
        self.active && self.is_open_for(project)
    }
    pub(crate) fn prune(&mut self, projects: &[Project]) {
        if self
            .view
            .as_ref()
            .is_some_and(|view| !projects.iter().any(|project| owns(&view.project, project)))
        {
            self.view = None;
            self.active = false;
        }
    }
}

fn owns(a: &Project, b: &Project) -> bool {
    a.id == b.id && a.path == b.path
}

/// A project contract may only tighten the client permission mode. It can
/// never grant Auto-approve to a project that asked, and read-only always wins.
pub(crate) fn permission_ceiling(
    mode: dunlin_domain::PermissionMode,
    autonomy: Option<ContractAutonomy>,
) -> dunlin_domain::PermissionMode {
    use dunlin_domain::PermissionMode;
    match autonomy {
        Some(ContractAutonomy::ReadOnly) => PermissionMode::ReadOnly,
        Some(ContractAutonomy::Ask) if mode == PermissionMode::AutoApprove => PermissionMode::Ask,
        _ => mode,
    }
}

fn actor() -> String {
    std::env::var("USER").unwrap_or_else(|_| "local user".into())
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn selected_path(snapshot: &Snapshot) -> Option<String> {
    if let Some(active) = &snapshot.active_feature
        && let Some(artifact) = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.path.starts_with(active))
            .or_else(|| {
                snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.path.contains(active))
            })
    {
        return Some(artifact.path.clone());
    }
    for kind in [
        Kind::Intent,
        Kind::Concept,
        Kind::Decision,
        Kind::AgentContract,
        Kind::Spec,
        Kind::Plan,
        Kind::Tasks,
    ] {
        if let Some(artifact) = snapshot.by_kind(kind).next() {
            return Some(artifact.path.clone());
        }
    }
    snapshot
        .artifacts
        .first()
        .map(|artifact| artifact.path.clone())
}

fn persistence_key(project_id: &str) -> String {
    format!("spec-persistence:{project_id}")
}

fn load_persistence(store: Option<&dunlin_storage::Store>, project: &Project) -> SpecPersistence {
    store
        .and_then(|store| {
            store
                .load_setting::<String>(&persistence_key(&project.id))
                .ok()
                .flatten()
        })
        .map(|value| SpecPersistence::parse(&value))
        .unwrap_or_default()
}

fn index_snapshot(store: Option<&dunlin_storage::Store>, project: &Project, snapshot: &Snapshot) {
    if let Some(store) = store
        && let Err(error) = store.replace_artifact_index(&project.id, &snapshot.artifacts)
    {
        crate::logging::warn(&format!("artifact index: {error}"));
    }
}

fn remote_candidate(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    if (lower.starts_with("intent/")
        || lower.starts_with("concepts/")
        || lower.starts_with("decisions/"))
        && lower.ends_with(".md")
    {
        return true;
    }
    if matches!(
        lower.as_str(),
        ".dunlin/agent-contract.md"
            | ".specify/memory/constitution.md"
            | ".specify/constitution.md"
    ) {
        return true;
    }
    let in_feature = lower.starts_with("specs/")
        || lower.starts_with(".specify/specs/")
        || lower.starts_with(".specify/features/");
    if !in_feature {
        return false;
    }
    let file = lower.rsplit('/').next().unwrap_or("");
    matches!(
        file,
        "spec.md" | "plan.md" | "tasks.md" | "research.md" | "data-model.md" | "quickstart.md"
    ) || (lower.contains("/checklists/") && lower.ends_with(".md"))
}

fn remote_relative(project: &Project, path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path
        .strip_prefix(&project.path)
        .unwrap_or(&path)
        .trim_start_matches('/');
    path.to_string()
}

fn flatten_remote(
    entries: &[dunlin_workspace::tree::FileEntry],
    output: &mut Vec<(String, String)>,
    depth: usize,
) {
    if depth > 64 || output.len() >= 20_000 {
        return;
    }
    for entry in entries {
        if entry.is_dir {
            flatten_remote(&entry.children, output, depth + 1);
        } else {
            output.push((entry.path.clone(), entry.name.clone()));
        }
    }
}

fn load_remote_view(
    project: &Project,
    backend: &WorkspaceBackend,
    approvals: Vec<dunlin_storage::ArtifactApproval>,
    evidence: Vec<dunlin_storage::TaskEvidence>,
    persistence: SpecPersistence,
) -> View {
    let tree: FileTree = match backend.tree() {
        Ok(tree) => tree,
        Err(error) => {
            return View {
                project: project.clone(),
                snapshot: Snapshot::default(),
                approvals,
                evidence,
                selected: None,
                persistence,
                inbox: false,
                trace: false,
                expanded_requirements: HashSet::new(),
                expanded_tasks: HashSet::new(),
                loading: false,
                error: Some(format!("Could not list relay artifacts: {error}")),
            };
        }
    };
    let mut warnings = Vec::new();
    if let Some(error) = &tree.error {
        warnings.push(format!("Relay tree: {error}"));
    }
    let mut entries = Vec::new();
    flatten_remote(&tree.entries, &mut entries, 0);
    entries.sort();
    entries.dedup_by(|a, b| a.0 == b.0);
    let mut candidates: Vec<(String, String)> = entries
        .iter()
        .filter_map(|(path, _)| {
            let relative = remote_relative(project, path);
            remote_candidate(&relative).then(|| (relative, path.clone()))
        })
        .collect();
    candidates.sort();
    candidates.truncate(256);
    let mut artifacts = Vec::new();
    let mut total = 0usize;
    for (relative, path) in candidates {
        if artifacts.len() >= 256 {
            warnings.push("Relay artifact discovery stopped after 256 files".into());
            break;
        }
        let file = match backend.read_file(&path) {
            Ok(file) => file,
            Err(error) => {
                warnings.push(format!("{relative}: relay read failed ({error})"));
                continue;
            }
        };
        if file.binary || file.bytes as usize > artifacts::MAX_ARTIFACT_BYTES {
            warnings.push(format!(
                "{relative}: binary or larger than {} KiB and not indexed",
                artifacts::MAX_ARTIFACT_BYTES / 1024
            ));
            continue;
        }
        if total + file.contents.len() > artifacts::MAX_TOTAL_BYTES {
            warnings.push("Relay artifact discovery exhausted its size budget".into());
            break;
        }
        total += file.contents.len();
        artifacts.push(artifacts::artifact_from_source(
            &relative,
            &file.contents,
            file.bytes,
            file.mtime_ms.min(i64::MAX as u64) as i64,
            None,
        ));
    }
    let active_feature = entries
        .iter()
        .find(|(path, _)| {
            let relative = remote_relative(project, path);
            relative == ".specify/feature.json" || relative == ".specify/feature"
        })
        .and_then(|(_, path)| backend.read_file(path).ok())
        .and_then(|file| artifacts::parse_active_feature(&file.contents));
    let snapshot = artifacts::snapshot_from_parts(
        project.path.clone(),
        artifacts,
        active_feature,
        Vec::new(),
        warnings,
    );
    let selected = selected_path(&snapshot);
    View {
        project: project.clone(),
        snapshot,
        approvals,
        evidence,
        selected,
        persistence,
        inbox: false,
        trace: false,
        expanded_requirements: HashSet::new(),
        expanded_tasks: HashSet::new(),
        loading: false,
        error: None,
    }
}

pub(crate) fn remote_snapshot(project: &Project, backend: &WorkspaceBackend) -> Snapshot {
    load_remote_view(
        project,
        backend,
        Vec::new(),
        Vec::new(),
        SpecPersistence::default(),
    )
    .snapshot
}

fn load_view(project: &Project, store: Option<&dunlin_storage::Store>) -> View {
    let snapshot = artifacts::discover(Path::new(&project.path));
    let approvals = store
        .and_then(|store| store.artifact_approvals(&project.id).ok())
        .unwrap_or_default();
    let evidence = store
        .and_then(|store| store.task_evidence(&project.id).ok())
        .unwrap_or_default();
    let persistence = load_persistence(store, project);
    match snapshot {
        Ok(snapshot) => {
            let selected = selected_path(&snapshot);
            index_snapshot(store, project, &snapshot);
            View {
                project: project.clone(),
                snapshot,
                approvals,
                evidence,
                selected,
                persistence,
                inbox: false,
                trace: false,
                expanded_requirements: HashSet::new(),
                expanded_tasks: HashSet::new(),
                loading: false,
                error: None,
            }
        }
        Err(error) => View {
            project: project.clone(),
            snapshot: Snapshot::default(),
            approvals,
            evidence: Vec::new(),
            selected: None,
            persistence,
            inbox: false,
            trace: false,
            expanded_requirements: HashSet::new(),
            expanded_tasks: HashSet::new(),
            loading: false,
            error: Some(error),
        },
    }
}

pub(crate) fn governance_enabled(plugins: &crate::plugins::State, project_id: &str) -> bool {
    plugins
        .packs
        .iter()
        .find(|pack| pack.manifest.id == "dunlin-governance")
        .is_some_and(|pack| plugins.is_enabled_for(Some(project_id), pack))
}

fn apply_governance_filter(
    view: &mut View,
    include_governance: bool,
    store: Option<&dunlin_storage::Store>,
) {
    if include_governance {
        return;
    }
    view.snapshot
        .artifacts
        .retain(|artifact| !matches!(artifact.kind, Kind::Concept | Kind::Decision));
    view.selected = view
        .selected
        .clone()
        .filter(|path| view.snapshot.get(path).is_some())
        .or_else(|| selected_path(&view.snapshot));
    index_snapshot(store, &view.project, &view.snapshot);
}

fn refresh_view(view: &mut View, store: Option<&dunlin_storage::Store>) {
    let previous = view.selected.clone();
    let previous_feature = view.snapshot.active_feature.clone();
    let snapshot = artifacts::discover(Path::new(&view.project.path));
    view.approvals = store
        .and_then(|store| store.artifact_approvals(&view.project.id).ok())
        .unwrap_or_default();
    view.evidence = store
        .and_then(|store| store.task_evidence(&view.project.id).ok())
        .unwrap_or_default();
    match snapshot {
        Ok(mut snapshot) => {
            snapshot.active_feature = previous_feature;
            snapshot.phases = artifacts::sdd_phases(&snapshot);
            view.selected = previous
                .filter(|path| snapshot.get(path).is_some())
                .or_else(|| selected_path(&snapshot));
            index_snapshot(store, &view.project, &snapshot);
            view.snapshot = snapshot;
            view.error = None;
        }
        Err(error) => {
            view.error = Some(error);
        }
    }
}

impl DunlinApp {
    pub(crate) fn open_spec_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.spec_tools_enabled {
            self.file_notice(
                "Spec, intent and agent-contract tools are disabled in Settings → General",
                cx,
            );
            return;
        }
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        let project = self.project().clone();
        if !project.trusted {
            self.file_notice(
                "Trust the project before reading its governing artifacts",
                cx,
            );
            return;
        }
        self.branch_ui.dismiss();
        self.remember_workspace_tab();
        self.project_settings_target = None;
        self.project_runtime_draft = None;
        self.group_settings_draft = None;
        self.context_menu = None;
        self.project_menu_open = false;
        self.settings_open = false;
        self.command_open = false;
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.git_history.active = false;
        if self.spec.decision_note.is_none() {
            self.spec.decision_note = Some(
                cx.new(|cx| InputState::new(window, cx).placeholder("Decision note (optional)")),
            );
        }
        if self.spec.is_open_for(&project) {
            self.spec.active = true;
            self.remember_workspace_tab();
            cx.notify();
            return;
        }
        if project.is_remote() {
            let backend = match self.backend() {
                Ok(backend) => backend,
                Err(error) => {
                    self.file_notice(error, cx);
                    return;
                }
            };
            let approvals = self
                .store
                .as_ref()
                .and_then(|store| store.artifact_approvals(&project.id).ok())
                .unwrap_or_default();
            let evidence = self
                .store
                .as_ref()
                .and_then(|store| store.task_evidence(&project.id).ok())
                .unwrap_or_default();
            let persistence = load_persistence(self.store.as_ref(), &project);
            self.spec.view = Some(View {
                project: project.clone(),
                snapshot: Snapshot::default(),
                approvals: approvals.clone(),
                evidence: evidence.clone(),
                selected: None,
                persistence,
                inbox: false,
                trace: false,
                expanded_requirements: HashSet::new(),
                expanded_tasks: HashSet::new(),
                loading: true,
                error: None,
            });
            self.spec.active = true;
            self.remember_workspace_tab();
            self.focus_handle.focus(window, cx);
            self.workbench
                .panes
                .opened(crate::workspace_panes::TabKey::Spec(project.id.clone()));
            let project_for_result = project.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        load_remote_view(
                            &project_for_result,
                            &backend,
                            approvals,
                            evidence,
                            persistence,
                        )
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if !this.spec.is_open_for(&project) {
                        return;
                    }
                    if let Some(store) = this.store.as_ref() {
                        index_snapshot(Some(store), &project, &result.snapshot);
                    }
                    let mut result = result;
                    apply_governance_filter(
                        &mut result,
                        governance_enabled(&this.plugins, &project.id),
                        this.store.as_ref(),
                    );
                    this.spec.view = Some(result);
                    this.spec.active = true;
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        let mut view = load_view(&project, self.store.as_ref());
        apply_governance_filter(
            &mut view,
            governance_enabled(&self.plugins, &project.id),
            self.store.as_ref(),
        );
        self.spec.view = Some(view);
        self.spec.active = true;
        self.remember_workspace_tab();
        self.workbench
            .panes
            .opened(crate::workspace_panes::TabKey::Spec(project.id.clone()));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_spec_workspace(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.spec.view {
            self.workbench
                .panes
                .closed(&crate::workspace_panes::TabKey::Spec(
                    view.project.id.clone(),
                ));
        }
        self.spec.active = false;
        self.spec.view = None;
        cx.notify();
    }

    pub(crate) fn refresh_spec_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_ref() else {
            return;
        };
        if !view.project.is_remote() {
            let view = self.spec.view.as_mut().unwrap();
            refresh_view(view, self.store.as_ref());
            apply_governance_filter(
                view,
                governance_enabled(&self.plugins, &view.project.id.clone()),
                self.store.as_ref(),
            );
            cx.notify();
            return;
        }
        let project = view.project.clone();
        let persistence = view.persistence;
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let approvals = self
            .store
            .as_ref()
            .and_then(|store| store.artifact_approvals(&project.id).ok())
            .unwrap_or_default();
        let evidence = self
            .store
            .as_ref()
            .and_then(|store| store.task_evidence(&project.id).ok())
            .unwrap_or_default();
        if let Some(view) = self.spec.view.as_mut() {
            view.loading = true;
            view.error = None;
        }
        let project_for_result = project.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    load_remote_view(
                        &project_for_result,
                        &backend,
                        approvals,
                        evidence,
                        persistence,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.spec.is_open_for(&project) {
                    return;
                }
                if let Some(store) = this.store.as_ref() {
                    index_snapshot(Some(store), &project, &result.snapshot);
                }
                let mut result = result;
                apply_governance_filter(
                    &mut result,
                    governance_enabled(&this.plugins, &project.id),
                    this.store.as_ref(),
                );
                this.spec.view = Some(result);
                this.spec.active = true;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn copy_decision_log(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_ref() else {
            return;
        };
        if view.approvals.is_empty() {
            self.set_notice("No local artifact decisions to export", cx);
            return;
        }
        let json = serde_json::to_string_pretty(&view.approvals).unwrap_or_default();
        cx.write_to_clipboard(ClipboardItem::new_string(json));
        self.set_notice(
            format!(
                "Copied {} hash-bound decision(s) to the clipboard. Commit messages and signed attestations remain later work.",
                view.approvals.len()
            ),
            cx,
        );
    }

    pub(crate) fn cycle_spec_persistence(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_mut() else {
            return;
        };
        let next = view.persistence.next();
        let project = view.project.clone();
        let Some(store) = self.store.as_ref() else {
            self.file_notice(
                "Local storage is unavailable; persistence model not saved",
                cx,
            );
            return;
        };
        if let Err(error) = store.save_setting(&persistence_key(&project.id), &next.key()) {
            self.file_notice(format!("Could not save the persistence model: {error}"), cx);
            return;
        }
        view.persistence = next;
        self.set_notice(
            format!(
                "Spec persistence model: {}. This affects drift warnings only; files are never rewritten.",
                next.label()
            ),
            cx,
        );
    }

    pub(crate) fn cycle_active_feature(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_mut() else {
            return;
        };
        let features = view.snapshot.features();
        if features.is_empty() {
            self.set_notice("No Spec Kit feature directories were detected", cx);
            return;
        }
        let mut options: Vec<Option<String>> = vec![None];
        options.extend(features.into_iter().map(Some));
        let current = options
            .iter()
            .position(|option| option.as_deref() == view.snapshot.active_feature.as_deref())
            .unwrap_or(0);
        let next = options[(current + 1) % options.len()].clone();
        view.snapshot.active_feature = next.clone();
        view.snapshot.phases = artifacts::sdd_phases(&view.snapshot);
        view.selected = if view.inbox {
            view.snapshot
                .by_kind(Kind::Intent)
                .next()
                .map(|artifact| artifact.path.clone())
        } else {
            selected_path(&view.snapshot)
        };
        self.set_notice(
            next.map(|feature| format!("Viewing feature: {feature}"))
                .unwrap_or_else(|| "Viewing all features".into()),
            cx,
        );
    }

    pub(crate) fn toggle_spec_trace(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_mut() else {
            return;
        };
        view.trace = !view.trace;
        if view.trace && view.expanded_requirements.is_empty() && view.expanded_tasks.is_empty() {
            for artifact in view.snapshot.scoped(Kind::Spec) {
                for requirement in &artifact.requirements {
                    view.expanded_requirements.insert(requirement.id.clone());
                }
            }
            for artifact in view.snapshot.scoped(Kind::Tasks) {
                for task in &artifact.tasks {
                    view.expanded_tasks.insert(task.id.clone());
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_intent_inbox(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_mut() else {
            return;
        };
        view.inbox = !view.inbox;
        view.selected = if view.inbox {
            view.snapshot
                .by_kind(Kind::Intent)
                .next()
                .map(|artifact| artifact.path.clone())
        } else {
            selected_path(&view.snapshot)
        };
        cx.notify();
    }

    pub(crate) fn start_intent_interview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.spec_tools_enabled {
            self.file_notice("Intent capture is disabled in Settings → General", cx);
            return;
        }
        let prompt = "Help me capture the intent for the current change. Interview me one question at a time about the problem, proposed outcome, affected users and systems, constraints, non-goals and open questions. Do not invent answers. When the answers are clear, write intent/YYYY-MM-DD-<slug>.md using the intent template and show it for review. Do not edit other files or commit without explicit approval.";
        let mut next = self.composer.read(cx).value().to_string();
        if !next.trim().is_empty() {
            next.push_str("\n\n");
        }
        next.push_str(prompt);
        self.composer
            .update(cx, |state, cx| state.set_value(next, window, cx));
        self.set_notice(
            "Added an intent-interview prompt. The agent must ask before writing the draft.",
            cx,
        );
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    pub(crate) fn capture_intent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.spec_tools_enabled {
            self.file_notice("Intent capture is disabled in Settings → General", cx);
            return;
        }
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        let project = self.project().clone();
        if !project.trusted || project.is_remote() {
            self.file_notice(
                "Intent capture currently requires a trusted local project",
                cx,
            );
            return;
        }
        let draft_text = self.composer.read(cx).value().to_string();
        let title = draft_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| {
                line.trim_start_matches('#')
                    .trim()
                    .chars()
                    .take(80)
                    .collect::<String>()
            })
            .filter(|line| !line.is_empty())
            .unwrap_or_else(|| "New intent".into());
        let draft = IntentDraft {
            title,
            author: actor(),
            problem: draft_text.chars().take(8000).collect(),
            outcome: String::new(),
            users: String::new(),
            constraints: String::new(),
            open_questions: String::new(),
        };
        let date = today();
        let contents = artifacts::render_intent(&draft, &date);
        let root = Path::new(&project.path);
        if let Err(error) = std::fs::create_dir_all(root.join(artifacts::INTENT_DIR)) {
            self.file_notice(format!("Could not create the intent folder: {error}"), cx);
            return;
        }
        let absolute = artifacts::unique_intent_path(root, &date, &draft.title);
        let Some(name) = absolute.file_name().and_then(|name| name.to_str()) else {
            self.file_notice("Could not choose an intent filename", cx);
            return;
        };
        let relative = format!("{}/{}", artifacts::INTENT_DIR, name);
        let result = std::fs::write(&absolute, contents)
            .map_err(|error| format!("Could not write the intent: {error}"))
            .and_then(|_| {
                std::fs::canonicalize(&absolute)
                    .map_err(|error| format!("Could not resolve the new intent: {error}"))
            });
        let path = match result {
            Ok(path) => path.to_string_lossy().to_string(),
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        if self.spec.is_open_for(&project) {
            self.refresh_spec_workspace(cx);
        }
        self.set_notice(
            format!("Created {relative} from the composer draft. Review it before committing."),
            cx,
        );
        self.spec.active = false;
        self.open_file(&path, window, cx);
    }

    pub(crate) fn open_agent_contract(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.spec_tools_enabled {
            self.file_notice("The agent contract is disabled in Settings → General", cx);
            return;
        }
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        let project = self.project().clone();
        if !project.trusted || project.is_remote() {
            self.file_notice(
                "The agent contract currently requires a trusted local project",
                cx,
            );
            return;
        }
        let root = Path::new(&project.path);
        let absolute = root.join(artifacts::AGENT_CONTRACT);
        if !absolute.is_file() {
            let Some(parent) = absolute.parent() else {
                self.file_notice("Could not resolve the contract folder", cx);
                return;
            };
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.file_notice(format!("Could not create .dunlin: {error}"), cx);
                return;
            }
            let contents = artifacts::render_agent_contract(&project.name, &today());
            if let Err(error) = std::fs::write(&absolute, contents) {
                self.file_notice(format!("Could not create the agent contract: {error}"), cx);
                return;
            }
        }
        let path = match std::fs::canonicalize(&absolute) {
            Ok(path) => path.to_string_lossy().to_string(),
            Err(error) => {
                self.file_notice(format!("Could not resolve the agent contract: {error}"), cx);
                return;
            }
        };
        if self.spec.is_open_for(&project) {
            self.refresh_spec_workspace(cx);
        }
        self.spec.active = false;
        self.open_file(&path, window, cx);
    }

    fn attach_head_evidence(&mut self, task_id: String, cx: &mut Context<Self>) {
        let project = self.project().clone();
        let context = match self.backend().and_then(|backend| backend.branch_context()) {
            Ok(context) => context,
            Err(error) => {
                self.file_notice(format!("Could not read HEAD evidence: {error}"), cx);
                return;
            }
        };
        let Some(head) = context.head else {
            self.file_notice("No HEAD commit is available; evidence was not recorded", cx);
            return;
        };
        let Some(store) = self.store.as_ref() else {
            self.file_notice(
                "Local storage is unavailable; evidence was not recorded",
                cx,
            );
            return;
        };
        if let Err(error) = store
            .set_task_evidence(
                &project.id,
                &task_id,
                "commit",
                &head,
                &format!(
                    "HEAD {} · branch {}",
                    &head[..head.len().min(12)],
                    context.current
                ),
            )
            .and_then(|_| {
                store.set_task_evidence(
                    &project.id,
                    &task_id,
                    "branch",
                    &context.current,
                    "branch observed with the attached HEAD",
                )
            })
        {
            self.file_notice(format!("Could not record task evidence: {error}"), cx);
            return;
        }
        if let Some(view) = self.spec.view.as_mut() {
            view.evidence = store.task_evidence(&view.project.id).unwrap_or_default();
        }
        self.set_notice(
            format!("Attached HEAD {} to {task_id}", &head[..head.len().min(12)]),
            cx,
        );
    }

    fn complete_task(&mut self, task_id: String, tasks_path: String, cx: &mut Context<Self>) {
        let project = self.project().clone();
        let worktree = self
            .subtask_worktrees
            .registry
            .entries
            .iter()
            .find(|entry| entry.project_id == project.id && entry.task_id == task_id)
            .cloned();
        let Some(worktree) = worktree else {
            self.file_notice(
                "Merge gate: create a task worktree for this T### before marking it complete",
                cx,
            );
            return;
        };
        let context = self.backend().and_then(|backend| backend.branch_context());
        let head = context
            .as_ref()
            .ok()
            .and_then(|context| context.head.clone())
            .unwrap_or_else(|| worktree.base.clone());
        let branch = context
            .as_ref()
            .map(|context| context.current.clone())
            .unwrap_or_else(|_| worktree.branch.clone());
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let file = match backend.read_file(&tasks_path) {
            Ok(file) => file,
            Err(error) => {
                self.file_notice(format!("Could not read tasks.md: {error}"), cx);
                return;
            }
        };
        let Some(updated) = check_task_line(&file.contents, &task_id) else {
            self.file_notice(
                format!("{task_id} is not an unchecked task in {tasks_path}; no file was changed"),
                cx,
            );
            return;
        };
        if let Err(error) = backend.write_file(&tasks_path, &updated, Some(file.mtime_ms)) {
            self.file_notice(format!("Could not mark {task_id} complete: {error}"), cx);
            return;
        }
        if let Some(store) = self.store.as_ref() {
            let _ = store.set_task_evidence(
                &project.id,
                &task_id,
                "commit",
                if head.is_empty() {
                    &worktree.base
                } else {
                    &head
                },
                &format!("completion evidence · branch {branch}"),
            );
            let _ = store.set_task_evidence(
                &project.id,
                &task_id,
                "worktree",
                &worktree.branch,
                &format!("worktree {}", worktree.path),
            );
        }
        self.set_notice(
            format!(
                "Marked {task_id} complete; worktree {} remains separate and was not merged",
                worktree.branch
            ),
            cx,
        );
        self.refresh_spec_workspace(cx);
    }

    fn open_artifact(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_ref() else {
            return;
        };
        let Some(artifact) = view.snapshot.get(path) else {
            self.file_notice(
                "The artifact is no longer indexed; refresh the spec workspace",
                cx,
            );
            return;
        };
        let target = artifact
            .absolute
            .clone()
            .unwrap_or_else(|| artifact.path.clone());
        self.spec.active = false;
        self.open_file(&target, window, cx);
    }

    fn approve_artifact(
        &mut self,
        path: String,
        accepted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (project_id, revision, description) = {
            let Some(view) = self.spec.view.as_ref() else {
                return;
            };
            let Some(artifact) = view.snapshot.get(&path) else {
                return;
            };
            let description = format!(
                "{}\n{path}\nrevision {}\n\nThis records a local {} decision bound to these exact bytes and the currently observed HEAD. It does not edit the file, commit it or send it to an agent.",
                artifact.title,
                artifact.short_revision(),
                if accepted { "approval" } else { "rejection" }
            );
            (
                view.project.id.clone(),
                artifact.revision.clone(),
                description,
            )
        };
        let note = self
            .spec
            .decision_note
            .as_ref()
            .map(|note| note.read(cx).value().to_string())
            .unwrap_or_default();
        let head_commit = self
            .backend()
            .ok()
            .and_then(|backend| backend.branch_context().ok())
            .and_then(|context| context.head)
            .unwrap_or_default();
        let receiver = window.prompt(
            if accepted {
                PromptLevel::Info
            } else {
                PromptLevel::Warning
            },
            if accepted {
                "Approve this governing artifact?"
            } else {
                "Reject this governing artifact?"
            },
            Some(&description),
            &["Cancel", if accepted { "Approve" } else { "Reject" }],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let Some(store) = this.store.as_ref() else {
                    this.file_notice("Local storage is unavailable; no decision was recorded", cx);
                    return;
                };
                let status = if accepted { "accepted" } else { "rejected" };
                if let Err(error) = store.set_artifact_approval(
                    &project_id,
                    &path,
                    &revision,
                    status,
                    &actor(),
                    &note,
                    &head_commit,
                ) {
                    this.file_notice(format!("Could not record the decision: {error}"), cx);
                    return;
                }
                if let Some(view) = this.spec.view.as_mut() {
                    view.approvals = store
                        .artifact_approvals(&view.project.id)
                        .unwrap_or_default();
                }
                this.set_notice(
                    format!(
                        "{} {path} at {}",
                        if accepted { "Approved" } else { "Rejected" },
                        &revision[..revision.len().min(12)]
                    ),
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_approval(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(store) = self.store.as_ref() else {
            self.file_notice("Local storage is unavailable", cx);
            return;
        };
        let Some(view) = self.spec.view.as_ref() else {
            return;
        };
        let project_id = view.project.id.clone();
        if let Err(error) = store.clear_artifact_approval(&project_id, &path) {
            self.file_notice(format!("Could not clear the decision: {error}"), cx);
            return;
        }
        if let Some(view) = self.spec.view.as_mut() {
            view.approvals = store
                .artifact_approvals(&view.project.id)
                .unwrap_or_default();
        }
        self.set_notice(format!("Cleared the local decision for {path}"), cx);
    }

    fn attach_artifact(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.spec.view.as_ref() else {
            return;
        };
        let Some(artifact) = view.snapshot.get(path) else {
            return;
        };
        let body: String = artifact.body.chars().take(4000).collect();
        let block = format!(
            "\n\n---\nGoverning artifact: {}\nKind: {}\nRevision: {}\n\n{}\n---\n",
            artifact.path,
            artifact.kind.label(),
            artifact.short_revision(),
            body.trim()
        );
        let mut next = self.composer.read(cx).value().to_string();
        next.push_str(&block);
        self.composer
            .update(cx, |state, cx| state.set_value(next, window, cx));
        self.set_notice(
            format!(
                "Attached {} at {}. Review the prompt before sending; context is never injected silently.",
                artifact.path,
                artifact.short_revision()
            ),
            cx,
        );
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    fn attach_next_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (phase_id, phase_label, feature, gate) = {
            let Some(view) = self.spec.view.as_ref() else {
                return;
            };
            let Some(phase) = view.snapshot.next_phase() else {
                self.set_notice(
                    "No missing or blocked phase remains. Review the artifacts and evidence.",
                    cx,
                );
                return;
            };
            if phase.id == "implement"
                && let Some(gate) = view.snapshot.implementation_gate()
            {
                self.file_notice(format!("Implementation gate is closed: {gate}"), cx);
                return;
            }
            (
                phase.id.clone(),
                phase.label.clone(),
                view.snapshot.active_feature.clone(),
                view.snapshot.implementation_gate(),
            )
        };
        let command_id = match phase_id.as_str() {
            "constitution" => "speckit-constitution",
            "specify" => "speckit-specify",
            "clarify" => "speckit-clarify",
            "plan" => "speckit-plan",
            "checklist" => "speckit-checklist",
            "tasks" => "speckit-tasks",
            "analyze" => "speckit-analyze",
            "implement" => "speckit-implement",
            "converge" => "speckit-converge",
            _ => "",
        };
        if command_id.is_empty() {
            self.file_notice(
                "This phase has no safe canned prompt. Edit the artifact or use the agent directly.",
                cx,
            );
            return;
        }
        self.plugins.load();
        let project_id = self.project().id.clone();
        let Some(prompt) = self.plugins.command_prompt_for(&project_id, command_id) else {
            self.file_notice(
                "Enable a methodology pack that provides this prompt in Settings → Extensions",
                cx,
            );
            return;
        };
        let feature = feature
            .as_deref()
            .map(|feature| format!("\nActive feature: {feature}\n"))
            .unwrap_or_default();
        let mut next = self.composer.read(cx).value().to_string();
        if !next.trim().is_empty() {
            next.push_str("\n\n");
        }
        next.push_str(prompt.trim_end());
        next.push_str(&feature);
        self.composer
            .update(cx, |state, cx| state.set_value(next, window, cx));
        self.set_notice(
            format!(
                "Added the {phase_label} prompt from the enabled pack.{} The agent still needs your approval for its actions.",
                gate
                    .map(|gate| format!(" Gate warning: {gate}."))
                    .unwrap_or_default()
            ),
            cx,
        );
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }
}

fn check_task_line(contents: &str, task_id: &str) -> Option<String> {
    let mut completed = false;
    let mut updated = String::with_capacity(contents.len());
    for line in contents.split_inclusive('\n') {
        if !completed && line.contains(task_id) && line.contains("- [ ]") {
            updated.push_str(&line.replacen("- [ ]", "- [x]", 1));
            completed = true;
        } else {
            updated.push_str(line);
        }
    }
    completed.then_some(updated)
}

fn stamp(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "unknown time".into())
}

fn status_color(status: Status, p: &Palette) -> Hsla {
    match status {
        Status::Accepted => p.green,
        Status::Rejected => p.red,
        Status::InReview => p.amber,
        Status::Proposed => p.blue,
        Status::Draft | Status::Deprecated | Status::Superseded | Status::Unknown => p.muted_fg,
    }
}

fn phase_color(state: PhaseState, p: &Palette) -> Hsla {
    match state {
        PhaseState::Complete => p.green,
        PhaseState::InProgress => p.blue,
        PhaseState::Blocked => p.red,
        PhaseState::Missing => p.muted_fg,
        PhaseState::Unknown => p.amber,
    }
}

fn effective(
    artifact: &Artifact,
    approvals: &[dunlin_storage::ArtifactApproval],
) -> (String, Status, Option<dunlin_storage::ArtifactApproval>) {
    if let Some(approval) = approvals.iter().find(|row| row.path == artifact.path) {
        if approval.revision == artifact.revision {
            let status = if approval.status == "rejected" {
                Status::Rejected
            } else {
                Status::Accepted
            };
            return (
                format!(
                    "{} locally · {}",
                    if status == Status::Rejected {
                        "rejected"
                    } else {
                        "approved"
                    },
                    stamp(approval.approved_at)
                ),
                status,
                Some(approval.clone()),
            );
        }
        return (
            "review needed · content changed since the decision".into(),
            Status::Unknown,
            Some(approval.clone()),
        );
    }
    (artifact.status.label().into(), artifact.status, None)
}

fn badge(label: impl Into<SharedString>, color: Hsla, p: &Palette) -> Div {
    div()
        .h(px(19.))
        .px(px(7.))
        .rounded(px(4.))
        .bg(p.muted)
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label.into())
}

fn section_label(text: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .px(px(10.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(9.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted_fg)
        .child(text.into())
}

fn row_text(id: String, text: String, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(10.))
        .py(px(3.))
        .text_size(px(11.5))
        .text_color(p.fg)
        .child(text)
}

struct IntentMetrics {
    total: usize,
    accepted: usize,
    rejected: usize,
    open: usize,
    avg_age_days: f32,
}

fn intent_metrics(view: &View) -> IntentMetrics {
    let today = chrono::Local::now().date_naive();
    let mut total = 0;
    let mut accepted = 0;
    let mut rejected = 0;
    let mut age_days = 0i64;
    let mut age_count = 0i64;
    for artifact in view.snapshot.by_kind(Kind::Intent) {
        total += 1;
        match effective(artifact, &view.approvals).1 {
            Status::Accepted => accepted += 1,
            Status::Rejected => rejected += 1,
            _ => {}
        }
        if let Some(created) = artifact
            .frontmatter
            .get("created")
            .and_then(|value| chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok())
        {
            age_days += (today - created).num_days().max(0);
            age_count += 1;
        }
    }
    IntentMetrics {
        total,
        accepted,
        rejected,
        open: total.saturating_sub(accepted + rejected),
        avg_age_days: if age_count > 0 {
            age_days as f32 / age_count as f32
        } else {
            0.
        },
    }
}

fn metric_card(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    color: Hsla,
    p: &Palette,
) -> Div {
    div()
        .min_w(px(112.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(7.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(
            div()
                .text_size(px(9.))
                .text_color(p.muted_fg)
                .child(label.into()),
        )
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value.into()),
        )
}

fn metrics_bar(
    metrics: &IntentMetrics,
    coverage: &artifacts::Coverage,
    drift: &[artifacts::DriftWarning],
    tasks: (usize, usize),
    session_cost: f32,
    p: &Palette,
) -> Stateful<Div> {
    let mut bar = div()
        .id("spec-metrics")
        .flex_shrink_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(9.))
        .border_b_1()
        .border_color(p.border)
        .child(metric_card("Intents", metrics.total.to_string(), p.fg, p))
        .child(metric_card(
            "Accepted",
            format!("{}/{}", metrics.accepted, metrics.total),
            p.green,
            p,
        ))
        .child(metric_card(
            "Open review",
            metrics.open.to_string(),
            if metrics.open > 0 {
                p.amber
            } else {
                p.muted_fg
            },
            p,
        ))
        .child(metric_card(
            "Rejected",
            metrics.rejected.to_string(),
            if metrics.rejected > 0 {
                p.red
            } else {
                p.muted_fg
            },
            p,
        ))
        .child(metric_card(
            "Avg intent age",
            if metrics.total == 0 {
                "—".into()
            } else {
                format!("{:.1} d", metrics.avg_age_days)
            },
            p.fg,
            p,
        ))
        .child(metric_card(
            "Requirement coverage",
            format!("{:.0}%", coverage.percent()),
            if coverage.uncovered.is_empty() {
                p.green
            } else {
                p.amber
            },
            p,
        ))
        .child(metric_card(
            "Tasks",
            format!("{}/{}", tasks.0, tasks.1),
            if tasks.1 > 0 && tasks.0 == tasks.1 {
                p.green
            } else {
                p.fg
            },
            p,
        ))
        .child(metric_card(
            "Drift",
            drift.len().to_string(),
            if drift.is_empty() { p.green } else { p.red },
            p,
        ));
    if session_cost > 0. {
        bar = bar.child(metric_card(
            "Observed session cost",
            format!("${session_cost:.2}"),
            p.muted_fg,
            p,
        ));
    }
    bar
}

fn drift_bar(
    drift: &[artifacts::DriftWarning],
    coverage: &artifacts::Coverage,
    p: &Palette,
) -> Stateful<Div> {
    let mut bar = div()
        .id("spec-drift")
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(16.))
        .py(px(8.))
        .border_b_1()
        .border_color(p.border)
        .bg(if drift.is_empty() {
            p.subtle
        } else {
            p.amber_soft
        })
        .child(
            div()
                .text_size(px(9.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if drift.is_empty() {
                    p.muted_fg
                } else {
                    p.amber
                })
                .child(format!(
                    "DRIFT & COVERAGE · {:.0}% requirements linked · {} warning(s)",
                    coverage.percent(),
                    drift.len()
                )),
        );
    for warning in drift.iter().take(6) {
        bar = bar.child(
            div()
                .text_size(px(10.5))
                .text_color(p.fg)
                .child(format!("· {}", warning.message)),
        );
    }
    if drift.len() > 6 {
        bar = bar.child(
            div()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child(format!("… and {} more", drift.len() - 6)),
        );
    }
    bar
}

fn extensions_bar(extensions: &[artifacts::Extension], p: &Palette) -> Stateful<Div> {
    let mut bar = div()
        .id("spec-extensions")
        .flex_shrink_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(16.))
        .py(px(7.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(9.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted_fg)
                .child("SPEC KIT EXTENSIONS"),
        );
    for extension in extensions.iter().take(12) {
        let mut label = extension.id.clone();
        if !extension.version.is_empty() {
            label.push_str(&format!(" v{}", extension.version));
        }
        if !extension.commands.is_empty() {
            label.push_str(&format!(" · {} cmd", extension.commands.len()));
        }
        bar = bar.child(badge(label, p.violet, p));
    }
    bar
}

fn collect_tool_text(items: &[ChatItem], output: &mut Vec<(String, String)>, depth: usize) {
    if depth > 16 || output.len() >= 200 {
        return;
    }
    for item in items {
        match item {
            ChatItem::Tool { view, .. } => output.push((view.title.clone(), view.detail.clone())),
            ChatItem::Child { items, .. } => collect_tool_text(items, output, depth + 1),
            _ => {}
        }
    }
}

/// Heuristic runtime audit: match contract boundary keywords against observed
/// tool titles/details. This is evidence for a human, not a violation verdict.
fn contract_audit(this: &DunlinApp, view: &View) -> Vec<String> {
    let Some(contract) = view.snapshot.by_kind(Kind::AgentContract).next() else {
        return Vec::new();
    };
    let summary = artifacts::contract_summary(contract);
    let mut keywords: Vec<String> = [
        "push",
        "force",
        "delete",
        "secret",
        "credential",
        "network",
        "publish",
        "merge",
        "rewrite",
        "transmit",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for boundary in &summary.boundaries {
        for word in boundary.split_whitespace() {
            let word = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase();
            if word.len() >= 5
                && !matches!(
                    word.as_str(),
                    "never" | "without" | "before" | "explicit" | "approval" | "should"
                )
                && !keywords.contains(&word)
            {
                keywords.push(word);
            }
        }
    }
    let mut tools = Vec::new();
    collect_tool_text(&this.harness.transcript, &mut tools, 0);
    let mut findings = Vec::new();
    for (title, detail) in tools.iter().take(120) {
        let text = format!("{title} {detail}").to_ascii_lowercase();
        for keyword in &keywords {
            if text.contains(keyword) {
                findings.push(format!(
                    "{} · matched boundary keyword `{keyword}`",
                    title.chars().take(120).collect::<String>()
                ));
                break;
            }
        }
    }
    findings.sort();
    findings.dedup();
    findings.truncate(8);
    findings
}

fn policy_lint(project: &Project, autonomy: ContractAutonomy) -> Vec<String> {
    if project.is_remote()
        || !matches!(autonomy, ContractAutonomy::ReadOnly | ContractAutonomy::Ask)
    {
        return Vec::new();
    }
    let path = Path::new(&project.path).join("AGENTS.md");
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return Vec::new();
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 256 * 1024 {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lower = text.to_ascii_lowercase();
    let mut warnings = Vec::new();
    if lower.contains("auto-approve")
        || lower.contains("auto approve")
        || lower.contains("autonomous")
    {
        warnings.push(
            "AGENTS.md suggests autonomous/auto-approved operation, but the agent contract only allows ask/read-only. The client enforces the contract; resolve the instruction conflict."
                .into(),
        );
    }
    warnings
}

fn trace_panel(
    this: &DunlinApp,
    view: &View,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> Stateful<Div> {
    let specs: Vec<_> = view.snapshot.scoped(Kind::Spec).collect();
    let tasks: Vec<_> = view
        .snapshot
        .scoped(Kind::Tasks)
        .flat_map(|artifact| artifact.tasks.iter())
        .collect();
    let mut panel = div()
        .id("spec-trace-panel")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Traceability · requirement → task → evidence"),
                )
                .child(
                    dunlin_ui::button("trace-expand-all")
                        .ghost()
                        .small()
                        .label("Expand all")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(view) = this.spec.view.as_mut() {
                                for artifact in view.snapshot.scoped(Kind::Spec) {
                                    for requirement in &artifact.requirements {
                                        view.expanded_requirements.insert(requirement.id.clone());
                                    }
                                }
                                for artifact in view.snapshot.scoped(Kind::Tasks) {
                                    for task in &artifact.tasks {
                                        view.expanded_tasks.insert(task.id.clone());
                                    }
                                }
                            }
                            cx.notify();
                        })),
                )
                .child(
                    dunlin_ui::button("trace-collapse-all")
                        .ghost()
                        .small()
                        .label("Collapse all")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(view) = this.spec.view.as_mut() {
                                view.expanded_requirements.clear();
                                view.expanded_tasks.clear();
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("Links are indexed evidence, not proof. A requirement is linked when a task mentions its ID; commits, worktrees and sessions are attached explicitly or recorded by the completion gate."),
        );
    let mut linked_ids = std::collections::HashSet::new();
    for spec in &specs {
        for requirement in &spec.requirements {
            linked_ids.insert(requirement.id.clone());
            let expanded = view.expanded_requirements.contains(&requirement.id);
            let linked: Vec<_> = tasks
                .iter()
                .filter(|task| task.requirements.iter().any(|id| id == &requirement.id))
                .collect();
            let color = if linked.is_empty() { p.amber } else { p.green };
            panel = panel.child(
                div()
                    .id(format!("trace-req:{}", requirement.id))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .hover(move |row| row.bg(p.hover))
                    .on_click(cx.listener({
                        let id = requirement.id.clone();
                        move |this, _, _, cx| {
                            if let Some(view) = this.spec.view.as_mut()
                                && !view.expanded_requirements.remove(&id)
                            {
                                view.expanded_requirements.insert(id.clone());
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(p.muted_fg),
                    )
                    .child(div().size(px(7.)).rounded_full().bg(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{} · {}", requirement.id, requirement.text)),
                            )
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .text_color(p.muted_fg)
                                    .child(format!(
                                        "{} task(s) · {} evidence row(s)",
                                        linked.len(),
                                        view.evidence
                                            .iter()
                                            .filter(|evidence| linked
                                                .iter()
                                                .any(|task| { task.id == evidence.task_id }))
                                            .count()
                                    )),
                            ),
                    ),
            );
            if expanded {
                if linked.is_empty() {
                    panel = panel.child(
                        div()
                            .pl(px(34.))
                            .text_size(px(10.5))
                            .text_color(p.amber)
                            .child("No task references this requirement"),
                    );
                }
                for task in linked {
                    let task_expanded = view.expanded_tasks.contains(&task.id);
                    let worktree = this
                        .subtask_worktrees
                        .registry
                        .entries
                        .iter()
                        .find(|entry| {
                            entry.project_id == view.project.id && entry.task_id == task.id
                        });
                    let evidence: Vec<_> = view
                        .evidence
                        .iter()
                        .filter(|evidence| evidence.task_id == task.id)
                        .collect();
                    panel = panel.child(
                        div()
                            .id(format!("trace-task:{}:{}", requirement.id, task.id))
                            .pl(px(26.))
                            .pr(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .hover(move |row| row.bg(p.hover))
                            .on_click(cx.listener({
                                let id = task.id.clone();
                                move |this, _, _, cx| {
                                    if let Some(view) = this.spec.view.as_mut()
                                        && !view.expanded_tasks.remove(&id)
                                    {
                                        view.expanded_tasks.insert(id.clone());
                                    }
                                    cx.notify();
                                }
                            }))
                            .child(
                                Icon::new(if task_expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(11.))
                                .text_color(p.muted_fg),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(if task.done { p.green } else { p.fg })
                                    .child(format!(
                                        "{} {} · {}{}",
                                        if task.done { "[x]" } else { "[ ]" },
                                        task.id,
                                        task.text,
                                        if worktree.is_some() {
                                            " · worktree"
                                        } else {
                                            " · worktree required"
                                        }
                                    )),
                            ),
                    );
                    if task_expanded {
                        if let Some(worktree) = worktree {
                            panel = panel.child(
                                div()
                                    .pl(px(52.))
                                    .text_size(px(9.5))
                                    .text_color(p.muted_fg)
                                    .child(format!(
                                        "worktree · {} · {}",
                                        worktree.branch, worktree.path
                                    )),
                            );
                        }
                        for evidence in evidence.iter().take(5) {
                            panel = panel.child(
                                div()
                                    .pl(px(52.))
                                    .text_size(px(9.5))
                                    .text_color(p.muted_fg)
                                    .child(format!(
                                        "evidence · {} {}",
                                        evidence.kind,
                                        &evidence.reference[..evidence.reference.len().min(90)]
                                    )),
                            );
                        }
                        let linked_sessions: Vec<_> = this
                            .sessions
                            .iter()
                            .filter(|session| {
                                session.project_id == view.project.id
                                    && worktree.is_some_and(|worktree| {
                                        session.branch.as_deref() == Some(worktree.branch.as_str())
                                    })
                            })
                            .take(2)
                            .collect();
                        for session in linked_sessions {
                            panel = panel.child(
                                div()
                                    .pl(px(52.))
                                    .text_size(px(9.5))
                                    .text_color(p.muted_fg)
                                    .child(format!(
                                        "session · {} · ${:.2}",
                                        session.id, session.cost_usd
                                    )),
                            );
                        }
                    }
                }
            }
        }
    }
    let unlinked: Vec<_> = tasks
        .iter()
        .filter(|task| task.requirements.iter().all(|id| !linked_ids.contains(id)))
        .collect();
    if !unlinked.is_empty() {
        panel = panel.child(section_label("UNLINKED TASKS", p));
        for task in unlinked.iter().take(40) {
            panel = panel.child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "{} {} · {}",
                        if task.done { "[x]" } else { "[ ]" },
                        task.id,
                        task.text
                    )),
            );
        }
    }
    if specs.is_empty() && tasks.is_empty() {
        panel = panel.child(
            div()
                .p(px(12.))
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("No requirements or tasks are indexed for this feature."),
        );
    }
    panel
}

fn artifact_list(view: &View, p: &Palette, cx: &mut Context<DunlinApp>) -> Stateful<Div> {
    let mut list = div()
        .id("spec-artifact-list")
        .w(px(290.))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(p.border)
        .bg(p.panel);
    let kinds: &[Kind] = if view.inbox {
        &[Kind::Intent]
    } else {
        &[
            Kind::Intent,
            Kind::Concept,
            Kind::Decision,
            Kind::AgentContract,
            Kind::Constitution,
            Kind::Spec,
            Kind::Plan,
            Kind::Tasks,
            Kind::Checklist,
            Kind::Other,
        ]
    };
    for &kind in kinds {
        let rows: Vec<_> = view.snapshot.by_kind(kind).collect();
        if rows.is_empty() {
            continue;
        }
        list = list.child(section_label(
            format!("{} · {}", kind.label(), rows.len()),
            p,
        ));
        for artifact in rows {
            let path = artifact.path.clone();
            let selected = view.selected.as_deref() == Some(artifact.path.as_str());
            let (label, status, _) = effective(artifact, &view.approvals);
            let color = status_color(status, p);
            let title = artifact.title.clone();
            let subtitle = artifact.path.clone();
            list = list.child(
                div()
                    .id(format!("spec-artifact:{}", artifact.path))
                    .mx(px(6.))
                    .px(px(8.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(p.selection))
                    .hover(move |row| row.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(view) = this.spec.view.as_mut() {
                            view.selected = Some(path.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(6.)).rounded_full().bg(color).flex_shrink_0())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(12.))
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .truncate()
                            .child(format!("{subtitle} · {label}")),
                    ),
            );
        }
    }
    list
}

fn phase_strip(snapshot: &Snapshot, p: &Palette, cx: &mut Context<DunlinApp>) -> Stateful<Div> {
    let mut strip = div()
        .id("spec-phases")
        .flex_shrink_0()
        .overflow_x_scroll()
        .flex()
        .gap(px(8.))
        .px(px(16.))
        .py(px(12.))
        .border_b_1()
        .border_color(p.border);
    for phase in &snapshot.phases {
        let color = phase_color(phase.state, p);
        let path = phase.path.clone();
        let label = phase.label.clone();
        let detail = phase.detail.clone();
        let state = phase.state.label();
        let mut card = div()
            .id(format!("spec-phase:{}", phase.id))
            .min_w(px(158.))
            .max_w(px(220.))
            .flex_shrink_0()
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .flex()
            .flex_col()
            .gap(px(3.))
            .when(path.is_some(), |card| {
                card.cursor_pointer().hover(move |card| card.bg(p.hover))
            });
        if let Some(path) = path {
            card = card.on_click(cx.listener(move |this, _, window, cx| {
                this.open_artifact(&path, window, cx);
            }));
        }
        strip = strip.child(
            card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(div().size(px(6.)).rounded_full().bg(color))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(div().text_size(px(9.)).text_color(color).child(state)),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(p.muted_fg)
                    .max_h(px(30.))
                    .overflow_hidden()
                    .child(detail),
            ),
        );
    }
    strip
}

fn detail_panel(
    this: &DunlinApp,
    artifact: &Artifact,
    view: &View,
    note: Option<&Entity<InputState>>,
    p: &Palette,
    cx: &mut Context<DunlinApp>,
) -> Stateful<Div> {
    let (label, status, approval) = effective(artifact, &view.approvals);
    let color = status_color(status, p);
    let path = artifact.path.clone();
    let open_path = artifact.path.clone();
    let attach_path = artifact.path.clone();
    let approve_path = artifact.path.clone();
    let reject_path = artifact.path.clone();
    let clear_path = artifact.path.clone();
    let mut panel = div()
        .id("spec-detail")
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_y_scroll()
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(10.));
    panel =
        panel
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.fg)
                                    .child(artifact.title.clone()),
                            )
                            .child(div().text_size(px(10.5)).text_color(p.muted_fg).child(
                                format!(
                                    "{} · {} · revision {}",
                                    artifact.path,
                                    artifact.kind.label(),
                                    artifact.short_revision()
                                ),
                            )),
                    )
                    .child(badge(label.clone(), color, p)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(
                        dunlin_ui::button(format!("spec-open:{path}"))
                            .small()
                            .label("Open")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_artifact(&open_path, window, cx);
                            })),
                    )
                    .child(
                        dunlin_ui::button(format!("spec-attach:{path}"))
                            .ghost()
                            .small()
                            .label("Attach to composer")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.attach_artifact(&attach_path, window, cx);
                            })),
                    )
                    .child(
                        dunlin_ui::button(format!("spec-approve:{path}"))
                            .ghost()
                            .small()
                            .label("Approve…")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.approve_artifact(approve_path.clone(), true, window, cx);
                            })),
                    )
                    .child(
                        dunlin_ui::button(format!("spec-reject:{path}"))
                            .ghost()
                            .small()
                            .label("Reject…")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.approve_artifact(reject_path.clone(), false, window, cx);
                            })),
                    )
                    .when(approval.is_some(), |row| {
                        row.child(
                            dunlin_ui::button(format!("spec-clear:{path}"))
                                .ghost()
                                .small()
                                .label("Clear decision")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.clear_approval(clear_path.clone(), cx);
                                })),
                        )
                    }),
            );
    if let Some(note) = note {
        panel =
            panel.child(div().text_size(px(10.)).text_color(p.muted_fg).child(
                "Decision note · stored with the next approve/reject, never sent to the agent",
            ));
        panel = panel.child(Input::new(note).small());
    }
    if let Some(approval) = &approval {
        let stale = approval.revision != artifact.revision;
        panel = panel.child(
            div()
                .p(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(if stale { p.amber } else { p.border })
                .bg(if stale { p.amber_soft } else { p.subtle })
                .text_size(px(10.5))
                .text_color(if stale { p.amber } else { p.muted_fg })
                .child(format!(
                    "Local {} by {} at {} · HEAD {} · bound to revision {}{}",
                    approval.status,
                    approval.actor,
                    stamp(approval.approved_at),
                    if approval.commit.is_empty() {
                        "unknown"
                    } else {
                        approval.commit.get(..12).unwrap_or(&approval.commit)
                    },
                    approval.revision.get(..12).unwrap_or(&approval.revision),
                    if stale {
                        format!(" · current revision is {}", artifact.short_revision())
                    } else {
                        String::new()
                    }
                )),
        );
    }
    if let Some(gate) = view.snapshot.implementation_gate() {
        panel = panel.child(
            div()
                .p(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(p.amber)
                .bg(p.amber_soft)
                .text_size(px(10.5))
                .text_color(p.amber)
                .child(format!("Implementation gate: {gate}")),
        );
    }
    let (done, total) = artifact.task_progress();
    let (checked, check_total) = artifact.checklist_progress();
    let mut facts = Vec::new();
    if !artifact.requirements.is_empty() {
        facts.push(format!("{} requirement(s)", artifact.requirements.len()));
    }
    if total > 0 {
        facts.push(format!("{done}/{total} tasks"));
    }
    if check_total > 0 {
        facts.push(format!("{checked}/{check_total} checklist items"));
    }
    if !artifact.open_questions.is_empty() {
        facts.push(format!(
            "{} open question(s)",
            artifact.open_questions.len()
        ));
    }
    if let Some(author) = &artifact.author {
        facts.push(format!("author {author}"));
    }
    if !facts.is_empty() {
        panel = panel.child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child(facts.join(" · ")),
        );
    }
    if !artifact.requirements.is_empty() {
        let linked: HashSet<&str> = view
            .snapshot
            .scoped(Kind::Tasks)
            .flat_map(|tasks| tasks.tasks.iter())
            .flat_map(|task| task.requirements.iter())
            .map(String::as_str)
            .collect();
        panel = panel.child(section_label("Requirements · task coverage", p));
        for requirement in artifact.requirements.iter().take(12) {
            let covered = linked.contains(requirement.id.as_str());
            panel = panel.child(row_text(
                format!("spec-req:{}", requirement.id),
                format!(
                    "{}  {} · {}",
                    requirement.id,
                    requirement.text,
                    if covered { "linked task" } else { "NO TASK" }
                ),
                p,
            ));
        }
    }
    if !artifact.tasks.is_empty() {
        panel = panel.child(section_label("Tasks · worktree merge gate", p));
        for task in artifact.tasks.iter().take(20) {
            let worktree = this
                .subtask_worktrees
                .registry
                .entries
                .iter()
                .find(|entry| entry.project_id == view.project.id && entry.task_id == task.id);
            let evidence: Vec<_> = view
                .evidence
                .iter()
                .filter(|evidence| evidence.task_id == task.id)
                .collect();
            let suffix = if let Some(worktree) = worktree {
                format!(
                    " · worktree {} · evidence {}",
                    worktree.branch,
                    evidence.len()
                )
            } else if task.done {
                " · done without worktree record".to_string()
            } else {
                " · worktree required".to_string()
            };
            panel = panel.child(row_text(
                format!("spec-task:{}", task.id),
                format!(
                    "{} {} {}{}{}",
                    if task.done { "[x]" } else { "[ ]" },
                    task.id,
                    task.text,
                    if task.parallel { " · parallel" } else { "" },
                    suffix
                ),
                p,
            ));
            for evidence in evidence.iter().take(3) {
                panel = panel.child(
                    div()
                        .pl(px(22.))
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!(
                            "evidence · {} {}",
                            evidence.kind,
                            &evidence.reference[..evidence.reference.len().min(80)]
                        )),
                );
            }
            if !task.done {
                let task_label = format!("{} {}", task.id, task.text);
                let branch_label = task.id.to_ascii_lowercase();
                let task_id = task.id.clone();
                let tasks_path = artifact.path.clone();
                let mut actions = div().flex().flex_wrap().gap(px(5.)).pl(px(22.));
                if worktree.is_none() {
                    actions = actions.child(
                        dunlin_ui::button(format!("spec-worktree:{}", task.id))
                            .ghost()
                            .small()
                            .label("Create worktree · gate")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.create_task_worktree(&task_label, &branch_label, window, cx);
                            })),
                    );
                } else {
                    let attach_id = task_id.clone();
                    actions = actions.child(
                        dunlin_ui::button(format!("spec-attach-evidence:{}", task.id))
                            .ghost()
                            .small()
                            .label("Attach HEAD")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.attach_head_evidence(attach_id.clone(), cx)
                            })),
                    );
                    let complete_id = task_id.clone();
                    let complete_path = tasks_path.clone();
                    actions = actions.child(
                        dunlin_ui::button(format!("spec-complete:{}", task.id))
                            .ghost()
                            .small()
                            .label("Complete…")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.complete_task(complete_id.clone(), complete_path.clone(), cx)
                            })),
                    );
                }
                panel = panel.child(actions);
            }
        }
        panel = panel.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(
                    dunlin_ui::button("spec-open-checks")
                        .ghost()
                        .small()
                        .label("Checks")
                        .on_click(cx.listener(|this, _, window, cx| this.open_checks(window, cx))),
                )
                .child(
                    dunlin_ui::button("spec-open-review")
                        .ghost()
                        .small()
                        .label("Task review")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_task_review(window, cx)),
                        ),
                ),
        );
    }
    if !artifact.open_questions.is_empty() {
        panel = panel.child(section_label("Open questions", p));
        for question in artifact.open_questions.iter().take(12) {
            panel = panel.child(row_text(
                format!("spec-question:{question}"),
                format!("· {question}"),
                p,
            ));
        }
    }
    if artifact.kind == Kind::AgentContract {
        let summary = artifacts::contract_summary(artifact);
        let autonomy = summary
            .autonomy
            .as_deref()
            .map(artifacts::classify_autonomy)
            .unwrap_or(ContractAutonomy::Unknown);
        panel = panel.child(section_label("Contract summary", p));
        if let Some(autonomy_text) = summary.autonomy {
            panel = panel.child(row_text(
                "spec-contract-autonomy".to_string(),
                format!("autonomy · {autonomy_text}"),
                p,
            ));
        }
        panel = panel.child(row_text(
            "spec-contract-ceiling".to_string(),
            format!("contract autonomy · {}", autonomy.label()),
            p,
        ));
        if let Ok(settings) = this.resolved_project_settings(&view.project) {
            let requested = settings.permission_mode.value;
            let effective = permission_ceiling(requested, Some(autonomy));
            panel = panel.child(row_text(
                "spec-contract-effective".to_string(),
                format!(
                    "effective permission · {} (project requests {})",
                    effective.label(),
                    requested.label()
                ),
                p,
            ));
            if effective != requested {
                panel = panel.child(
                    div()
                        .p(px(8.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(p.amber)
                        .bg(p.amber_soft)
                        .text_size(px(10.5))
                        .text_color(p.amber)
                        .child(format!(
                            "Policy conflict: the project requests {}, but the agent contract tightens it to {}. The client enforces the tighter mode; the harness still owns model behaviour.",
                            requested.label(),
                            effective.label()
                        )),
                );
            }
        }
        for boundary in summary.boundaries.iter().take(8) {
            panel = panel.child(row_text(
                format!("spec-contract-boundary:{boundary}"),
                format!("boundary · {boundary}"),
                p,
            ));
        }
        let findings = contract_audit(this, view);
        panel = panel.child(section_label("Runtime audit · heuristic", p));
        if findings.is_empty() {
            panel = panel.child(
                div()
                    .text_size(px(10.))
                    .text_color(p.muted_fg)
                    .child("No observed tool title/detail matched a boundary keyword. This is not proof of compliance."),
            );
        } else {
            for finding in &findings {
                panel = panel.child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.amber)
                        .child(format!("· {finding}")),
                );
            }
        }
        for warning in policy_lint(&view.project, autonomy) {
            panel = panel.child(
                div()
                    .p(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.red)
                    .bg(p.red_soft)
                    .text_size(px(10.5))
                    .text_color(p.red)
                    .child(warning),
            );
        }
        panel = panel.child(
            div()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child("Dunlin tightens the client permission mode from this contract: read-only wins and Auto-approve is downgraded to Ask. Runtime audit is keyword evidence, not model-behaviour enforcement."),
        );
        panel = panel.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(
                    dunlin_ui::button("spec-contract-audit")
                        .ghost()
                        .small()
                        .label("Ask agent to audit")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_pack_command(
                                "agent-contract",
                                "agent-contract-audit",
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    dunlin_ui::button("spec-contract-eval")
                        .ghost()
                        .small()
                        .label("Run manual eval")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_pack_command(
                                "agent-contract",
                                "agent-contract-eval",
                                window,
                                cx,
                            );
                        })),
                ),
        );
    }
    let preview: String = artifact.body.chars().take(16_000).collect();
    panel = panel.child(section_label("Document", p)).child(
        div()
            .id("spec-document")
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .p(px(12.))
            .child(TextView::markdown(
                format!("spec-preview:{}", artifact.path),
                preview,
            )),
    );
    if !view.snapshot.warnings.is_empty() {
        panel = panel.child(section_label("Warnings", p));
        for warning in view.snapshot.warnings.iter().take(6) {
            panel = panel.child(
                div()
                    .text_size(px(10.))
                    .text_color(p.amber)
                    .child(warning.clone()),
            );
        }
    }
    panel
}

fn empty_state(p: &Palette, cx: &mut Context<DunlinApp>) -> AnyElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .p(px(32.))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("No governing artifacts yet"),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_center()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child("Dunlin indexes intent/, .specify/ (Spec Kit) and .dunlin/agent-contract.md as ordinary files. Nothing is generated or rewritten automatically. Capture an intent from the composer, open the agent contract template, or point the workspace at an existing Spec Kit project."),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    dunlin_ui::button("spec-empty-intent")
                        .small()
                        .label("Capture intent…")
                        .on_click(cx.listener(|this, _, window, cx| this.capture_intent(window, cx))),
                )
                .child(
                    dunlin_ui::button("spec-empty-interview")
                        .ghost()
                        .small()
                        .label("Interview with agent")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_intent_interview(window, cx)
                        })),
                )
                .child(
                    dunlin_ui::button("spec-empty-contract")
                        .ghost()
                        .small()
                        .label("Create agent contract")
                        .on_click(cx.listener(|this, _, window, cx| this.open_agent_contract(window, cx))),
                )
                .child(
                    dunlin_ui::button("spec-empty-refresh")
                        .ghost()
                        .small()
                        .label("Refresh")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_spec_workspace(cx))),
                ),
        )
        .into_any_element()
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(view) = this.spec.view.as_ref() else {
        return div().into_any_element();
    };
    let dirty: Vec<String> = this
        .git
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect();
    let metrics = intent_metrics(view);
    let coverage = view.snapshot.coverage();
    let drift = view.snapshot.drift(&dirty, view.persistence);
    let tasks = view.snapshot.tasks();
    let session_cost: f32 = this
        .sessions
        .iter()
        .filter(|session| session.project_id == view.project.id)
        .map(|session| session.cost_usd as f32)
        .sum();
    let features = view.snapshot.features();
    let selected = view.selected.as_deref().or_else(|| {
        view.snapshot
            .artifacts
            .first()
            .map(|artifact| artifact.path.as_str())
    });
    let selected_artifact = selected.and_then(|path| view.snapshot.get(path));
    let mut root = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.bg)
        .text_color(p.fg);
    let mut header = div()
        .flex_shrink_0()
        .px(px(16.))
        .py(px(11.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(9.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Spec workspace"),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(format!("{} · {}", view.project.name, view.project.path)),
                ),
        );
    if let Some(feature) = &view.snapshot.active_feature {
        header = header.child(badge(format!("active · {feature}"), p.blue, &p));
    }
    header = header
        .child(div().flex_1())
        .child(
            dunlin_ui::button("spec-next-step")
                .ghost()
                .small()
                .label("Next step")
                .on_click(cx.listener(|this, _, window, cx| this.attach_next_step(window, cx))),
        )
        .child(
            dunlin_ui::button("spec-refresh")
                .ghost()
                .small()
                .label("Refresh")
                .on_click(cx.listener(|this, _, _, cx| this.refresh_spec_workspace(cx))),
        )
        .child(
            dunlin_ui::button("spec-inbox-toggle")
                .ghost()
                .small()
                .label(if view.inbox {
                    "Workflow"
                } else {
                    "Intent inbox"
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_intent_inbox(cx))),
        )
        .child(
            dunlin_ui::button("spec-trace-toggle")
                .ghost()
                .small()
                .label(if view.trace { "Workflow" } else { "Trace" })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_spec_trace(cx))),
        )
        .when(features.len() > 1, |header| {
            header.child(
                dunlin_ui::button("spec-feature-cycle")
                    .ghost()
                    .small()
                    .label(
                        view.snapshot
                            .active_feature
                            .as_deref()
                            .map(|feature| {
                                format!(
                                    "Feature: {}",
                                    feature.rsplit('/').next().unwrap_or(feature)
                                )
                            })
                            .unwrap_or_else(|| "All features".into()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_active_feature(cx))),
            )
        })
        .child(
            dunlin_ui::button("spec-persistence")
                .ghost()
                .small()
                .label(format!("Persistence: {}", view.persistence.label()))
                .on_click(cx.listener(|this, _, _, cx| this.cycle_spec_persistence(cx))),
        )
        .child(
            dunlin_ui::button("spec-copy-decisions")
                .ghost()
                .small()
                .label("Copy decisions")
                .on_click(cx.listener(|this, _, _, cx| this.copy_decision_log(cx))),
        )
        .child(
            dunlin_ui::button("spec-intent-interview")
                .ghost()
                .small()
                .label("Interview…")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.start_intent_interview(window, cx)
                })),
        )
        .child(
            dunlin_ui::button("spec-capture-intent")
                .small()
                .label("Capture intent…")
                .on_click(cx.listener(|this, _, window, cx| this.capture_intent(window, cx))),
        )
        .child(
            dunlin_ui::button("spec-agent-contract")
                .ghost()
                .small()
                .label("Agent contract")
                .on_click(cx.listener(|this, _, window, cx| this.open_agent_contract(window, cx))),
        )
        .child(
            dunlin_ui::button("spec-install-pack-skill")
                .ghost()
                .small()
                .label("Install Spec Kit skill")
                .on_click(cx.listener(|this, _, _, cx| {
                    let target = Target::from_harness(&this.session_harness());
                    let project = this.project().clone();
                    if project.is_remote() || !project.trusted {
                        this.file_notice(
                            "The Spec Kit skill currently installs into local trusted projects only",
                            cx,
                        );
                        return;
                    }
                    this.install_catalog_skill(
                        SPEC_KIT_SKILL,
                        target,
                        Scope::Project,
                        &project.id,
                        Some(std::path::PathBuf::from(&project.path)),
                        cx,
                    );
                    if this.skill_installed_for(SPEC_KIT_SKILL, target, &project.id) {
                        this.set_notice(
                            "Installed the Spec-driven development skill for the active harness in this project",
                            cx,
                        );
                    } else {
                        this.file_notice(
                            "The active harness has no known project skills directory; open Settings → Skills to install the bundled skill manually",
                            cx,
                        );
                    }
                })),
        )
        .child(
            dunlin_ui::button("spec-extensions")
                .ghost()
                .small()
                .label("Extensions")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings_open = true;
                    this.settings_tab = SettingsTab::Extensions;
                    cx.notify();
                })),
        );
    root = root.child(header);
    if view.loading {
        return root
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_color(p.muted_fg)
                    .child("Loading relay artifacts…")
                    .child(div().text_size(px(10.5)).child(
                        "Bounded tree read and artifact hashing; nothing is written to the relay.",
                    )),
            )
            .into_any_element();
    }
    if !view.snapshot.artifacts.is_empty() {
        root = root.child(metrics_bar(
            &metrics,
            &coverage,
            &drift,
            tasks,
            session_cost,
            &p,
        ));
    }
    if !drift.is_empty() {
        root = root.child(drift_bar(&drift, &coverage, &p));
    }
    if !view.snapshot.extensions.is_empty() {
        root = root.child(extensions_bar(&view.snapshot.extensions, &p));
    }
    if let Some(error) = &view.error {
        root = root.child(
            div()
                .m(px(16.))
                .p(px(12.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.red)
                .bg(p.red_soft)
                .text_color(p.red)
                .text_size(px(11.5))
                .child(error.clone()),
        );
    } else if view.snapshot.artifacts.is_empty() {
        root = root.child(empty_state(&p, cx));
    } else {
        root = root.child(phase_strip(&view.snapshot, &p, cx));
        let mut body = div().flex().flex_1().min_h(px(0.));
        if view.trace {
            body = body.child(trace_panel(this, view, &p, cx));
        } else {
            body = body.child(artifact_list(view, &p, cx));
            if let Some(artifact) = selected_artifact {
                body = body.child(detail_panel(
                    this,
                    artifact,
                    view,
                    this.spec.decision_note.as_ref(),
                    &p,
                    cx,
                ));
            } else {
                body = body.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.muted_fg)
                        .child("Select an artifact"),
                );
            }
        }
        root = root.child(body);
    }
    root.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        PhaseState, Project, Status, check_task_line, effective, load_view, permission_ceiling,
        policy_lint, remote_candidate, remote_relative,
    };
    use dunlin_artifacts::{ContractAutonomy, SpecPersistence};
    use dunlin_domain::PermissionMode;
    use std::fs;
    use tempfile::tempdir;

    fn project(root: &std::path::Path) -> Project {
        Project {
            id: "spec-test".into(),
            name: "Spec test".into(),
            path: root.to_string_lossy().to_string(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    fn write(root: &std::path::Path, path: &str, contents: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn selected_artifact_prefers_the_active_feature_over_intent() {
        let root = tempdir().unwrap();
        write(root.path(), "intent/2026-11-04-why.md", "# Why\n");
        write(
            root.path(),
            ".specify/feature.json",
            r#"{"feature_directory":"specs/001-x"}"#,
        );
        write(root.path(), "specs/001-x/spec.md", "# Spec\n");
        let project = project(root.path());
        let view = load_view(&project, None);
        assert_eq!(view.selected.as_deref(), Some("specs/001-x/spec.md"));
        assert_eq!(view.snapshot.active_feature.as_deref(), Some("specs/001-x"));
    }

    #[test]
    fn approvals_are_bound_to_the_exact_revision() {
        let root = tempdir().unwrap();
        write(
            root.path(),
            "intent/2026-11-04-why.md",
            "---\nstatus: draft\n---\n# Why\n",
        );
        let view = load_view(&project(root.path()), None);
        let artifact = view.snapshot.get("intent/2026-11-04-why.md").unwrap();
        let approval = dunlin_storage::ArtifactApproval {
            project_id: "spec-test".into(),
            path: artifact.path.clone(),
            revision: artifact.revision.clone(),
            status: "accepted".into(),
            actor: "reviewer".into(),
            note: String::new(),
            commit: String::new(),
            approved_at: 0,
        };
        let (label, status, _) = effective(artifact, std::slice::from_ref(&approval));
        assert!(label.contains("approved locally"));
        assert_eq!(status, Status::Accepted);
        let mut stale = approval;
        stale.revision = "different".into();
        let (label, status, _) = effective(artifact, &[stale]);
        assert!(label.contains("review needed"));
        assert_eq!(status, Status::Unknown);
    }

    #[test]
    fn task_completion_only_changes_one_matching_checkbox() {
        let source = "# Tasks\n\n- [ ] T001 First\n- [ ] T002 Second\n";
        let updated = check_task_line(source, "T002").unwrap();
        assert!(updated.contains("- [x] T002 Second"));
        assert!(updated.contains("- [ ] T001 First"));
        assert!(check_task_line(source, "T999").is_none());
        assert!(check_task_line("- [x] T001 Done\n", "T001").is_none());
    }

    #[test]
    fn policy_lint_detects_agents_conflict_and_skips_remote_projects() {
        let root = tempdir().unwrap();
        write(
            root.path(),
            "AGENTS.md",
            "# Agents\n\nThe agent is autonomous and may auto-approve tools.\n",
        );
        let local = project(root.path());
        assert!(!policy_lint(&local, ContractAutonomy::Ask).is_empty());
        assert!(policy_lint(&local, ContractAutonomy::Auto).is_empty());
        let mut remote = local.clone();
        remote.remote = Some(dunlin_domain::RemoteProject {
            host_id: "host".into(),
            project_id: "project".into(),
            path: "/srv/project".into(),
        });
        assert!(policy_lint(&remote, ContractAutonomy::Ask).is_empty());
    }

    #[test]
    fn remote_candidates_only_include_governing_documents() {
        assert!(remote_candidate("intent/2026-11-04-why.md"));
        assert!(remote_candidate("specs/001-x/spec.md"));
        assert!(remote_candidate("specs/001-x/checklists/requirements.md"));
        assert!(remote_candidate(".specify/memory/constitution.md"));
        assert!(remote_candidate(".dunlin/agent-contract.md"));
        assert!(!remote_candidate("src/main.rs"));
        assert!(!remote_candidate("specs/001-x/notes.md"));
        let mut project = project(std::path::Path::new("/tmp/project"));
        project.path = "/srv/project".into();
        assert_eq!(
            remote_relative(&project, "/srv/project/specs/001-x/spec.md"),
            "specs/001-x/spec.md"
        );
        assert_eq!(
            remote_relative(&project, "specs/001-x/spec.md"),
            "specs/001-x/spec.md"
        );
    }

    #[test]
    fn spec_tools_setting_defaults_on_and_round_trips_off() {
        let mut settings = crate::app::StoredSettings::default();
        assert!(settings.spec_tools_enabled);
        settings.spec_tools_enabled = false;
        let restored: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!restored.spec_tools_enabled);
        let legacy: crate::app::StoredSettings = serde_json::from_str(r#"{"dark":false}"#).unwrap();
        assert!(legacy.spec_tools_enabled);
    }

    #[test]
    fn persistence_setting_roundtrips_and_is_written_to_the_index() {
        let root = tempdir().unwrap();
        write(root.path(), "specs/001-x/spec.md", "# Spec\n");
        let project = project(root.path());
        let store = dunlin_storage::Store::open_in_memory().unwrap();
        store
            .save_setting(
                &format!("spec-persistence:{}", project.id),
                &"living".to_string(),
            )
            .unwrap();
        let view = load_view(&project, Some(&store));
        assert_eq!(view.persistence, SpecPersistence::Living);
        let index = store.artifact_index(&project.id).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].path, "specs/001-x/spec.md");
    }

    #[test]
    fn contract_only_tightens_the_client_permission_mode() {
        assert_eq!(
            permission_ceiling(PermissionMode::AutoApprove, Some(ContractAutonomy::Ask)),
            PermissionMode::Ask
        );
        assert_eq!(
            permission_ceiling(
                PermissionMode::AutoApprove,
                Some(ContractAutonomy::ReadOnly)
            ),
            PermissionMode::ReadOnly
        );
        assert_eq!(
            permission_ceiling(PermissionMode::Ask, Some(ContractAutonomy::ReadOnly)),
            PermissionMode::ReadOnly
        );
        assert_eq!(
            permission_ceiling(PermissionMode::ReadOnly, Some(ContractAutonomy::Auto)),
            PermissionMode::ReadOnly
        );
        assert_eq!(
            permission_ceiling(PermissionMode::AutoApprove, Some(ContractAutonomy::Unknown)),
            PermissionMode::AutoApprove
        );
    }

    #[test]
    fn implementation_gate_blocks_on_open_questions_and_checklists() {
        let root = tempdir().unwrap();
        write(
            root.path(),
            "specs/001-x/spec.md",
            "# Spec\n\n## Open questions\n- [ ] Who owns it?\n",
        );
        write(root.path(), "specs/001-x/tasks.md", "- [ ] T001 Do it\n");
        write(
            root.path(),
            "specs/001-x/checklists/requirements.md",
            "- [ ] Review\n",
        );
        let view = load_view(&project(root.path()), None);
        assert!(view.snapshot.implementation_gate().is_some());
        assert_eq!(
            view.snapshot
                .phases
                .iter()
                .find(|phase| phase.id == "implement")
                .unwrap()
                .state,
            PhaseState::Blocked
        );
    }
}
