"use strict";

// Layout study only: no network, storage, agents, processes or filesystem APIs.
const $ = (selector, parent = document) => parent.querySelector(selector);
const $$ = (selector, parent = document) => [
  ...parent.querySelectorAll(selector),
];
const escapeHtml = (text) =>
  String(text).replace(
    /[&<>"']/g,
    (character) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        character
      ],
  );
const capabilities = window.DUNLIN_CAPABILITIES;
const settings = window.DUNLIN_SETTINGS;
const root = document.documentElement;
const workspace = $("#workspace");
const narrow = matchMedia("(max-width: 860px)");
const inspectorSheet = matchMedia("(max-width: 1350px)");
const dark = matchMedia("(prefers-color-scheme: dark)");
const initialCode = `use dunlin_ui::*;
use crate::workspace_panes::WorkspacePanes;

// Illustrative buffer — not application source.
pub struct Workspace {
    panes: WorkspacePanes,
    context: ProjectContext,
}

impl Workspace {
    pub fn render(&self) -> impl IntoElement {
        workbench()
            // Keep independent document panes.
            .navigator(self.project_tree())
            .documents(self.panes.clone())
            .tools(self.tool_dock())
            .inspector(self.context.clone())
    }

    fn navigation(&self) -> Navigation {
        Navigation::new()
            .projects_and_groups()
            .files_and_sessions()
            .git_and_worktrees()
            .knowledge_and_specifications()
            .security_and_evidence()
    }
}

// Presentation changes. Capabilities stay.
`;
const initialMarkdown =
  "# Workspace refresh\n\nKeep the full Dunlin workbench.\n\n- Projects and nested groups\n- Independent editor and conversation panes\n- Git, Knowledge, Spec and Security\n- Terminal, Run, Agents and Checks\n\nMaterials frame the work; they never obscure it.\n";
const makeSession = (title, empty = false) => ({
  title,
  draft: "",
  messages: [],
  empty,
  mode: "Build",
  agent: "OpenCode",
});
const makeProject = (name, host, path, title) => ({
  name,
  host,
  path,
  selected: 0,
  code: initialCode,
  markdown: initialMarkdown,
  sessions: [makeSession(title), makeSession("Review the implementation")],
});
const projects = {
  dunlin: makeProject(
    "Dunlin",
    "This Mac",
    "~/Projects/dunlin",
    "Workspace refinement",
  ),
  relay: makeProject(
    "Relay",
    "This Mac",
    "~/Projects/relay",
    "Connection workflow",
  ),
  server: makeProject(
    "API Service",
    "Studio relay",
    "/srv/api-service",
    "Inspect remote changes",
  ),
  scratch: makeProject(
    "Research notes",
    "This Mac",
    "Scratch workspace",
    "Explore an idea",
  ),
};
let projectId = "dunlin";
let navigatorMode = "projects";
let lens = "chat";
let dockMode = "terminal";
let selectedDocument = "editor";
let selectedSettings = "appearance";
let gitTab = "changes";
let knowledgeTab = "objects";
let specTab = "workflow";
let terminalSplit = false;
let editCode = false;
let rawMarkdown = false;
const openedDocuments = ["editor", "markdown"];
const project = () => projects[projectId];
const session = () => project().sessions[project().selected];
const titles = {
  editor: "workbench.rs",
  markdown: "workspace-notes.md",
  overview: "Overview",
  git: "Source Control",
  knowledge: "Knowledge",
  spec: "Intent & Specifications",
  artifacts: "Artifacts & Evidence",
  security: "Security Audits",
  compare: "A/B Comparison",
  review: "Task Review",
  impact: "Rust Impact",
  servers: "Server Administration",
  search: "Project Search",
};
const announce = (message) => {
  $("#announcement").textContent = message;
};

function openDialog(id, opener = document.activeElement) {
  const dialog = $(id);
  if (!dialog.open) {
    opener?.focus({ preventScroll: true });
    dialog.showModal();
  }
}
function closeDialogs() {
  $$("dialog[open]")
    .reverse()
    .forEach((dialog) => dialog.close());
}
function syncLayout() {
  const navVisible = workspace.dataset.sidebar === "true";
  $("#navigator").inert = !navVisible;
  $("#navigator").setAttribute("aria-hidden", String(!navVisible));
  $("#toggle-sidebar").setAttribute("aria-expanded", String(navVisible));
  const covered =
    (narrow.matches && navVisible) ||
    (inspectorSheet.matches && workspace.dataset.inspector === "true");
  $(".workbench").inert = covered;
  $(".workbench").setAttribute("aria-hidden", String(covered));
}
function setSidebar(visible) {
  if (visible && narrow.matches) setInspector(false, false);
  workspace.dataset.sidebar = String(visible);
  syncLayout();
  if (visible && narrow.matches) $(".home-button").focus();
  if (!visible && $("#navigator").contains(document.activeElement))
    $("#toggle-sidebar").focus();
}
function setInspector(visible, focus = true) {
  if (visible && narrow.matches) workspace.dataset.sidebar = "false";
  workspace.dataset.inspector = String(visible);
  $("#inspector").hidden = !visible;
  $("#toggle-inspector").setAttribute("aria-expanded", String(visible));
  syncLayout();
  if (focus) $(visible ? "#close-inspector" : "#toggle-inspector").focus();
}
function setDock(mode = dockMode) {
  dockMode = mode;
  workspace.dataset.dock = "true";
  $("#toggle-dock").setAttribute("aria-expanded", "true");
  $$("[data-dock]").forEach((button) => {
    const active = button.dataset.dock === mode;
    button.setAttribute("aria-selected", String(active));
    button.tabIndex = active ? 0 : -1;
  });
  renderDock();
}
function setTheme() {
  const choice = $("#theme").value;
  root.dataset.theme =
    choice === "system" ? (dark.matches ? "dark" : "light") : choice;
}
function navRow(label, action, detail = "", className = "", symbol = "▱") {
  return `<button class="nav-row ${className}" ${action}><span class="nav-symbol" aria-hidden="true">${symbol}</span><span>${escapeHtml(label)}</span>${detail ? `<small>${escapeHtml(detail)}</small>` : ""}</button>`;
}
function renderNavigator() {
  $$("[data-nav]").forEach((button) => {
    const active = button.dataset.nav === navigatorMode;
    button.setAttribute("aria-selected", String(active));
    button.tabIndex = active ? 0 : -1;
  });
  let content;
  if (navigatorMode === "projects") {
    content = `<div class="section-label">PERSONAL · GROUP</div>${navRow("Dunlin", 'data-project="dunlin"', "Local", `indent ${projectId === "dunlin" ? "selected" : ""}`, "▣")}${navRow("ui-materials", 'data-preview="Subtask worktree: ui-materials"', "Worktree", "deep", "⑂")}${navRow("Relay", 'data-project="relay"', "Local", `indent ${projectId === "relay" ? "selected" : ""}`, "▣")}<div class="inline-nav-actions"><button data-preview="Group defaults and inheritance">Group settings</button><button data-preview="Project trust and runtime defaults">Project settings</button></div><div class="section-label">STUDIO · RELAY HOST</div>${navRow("API Service", 'data-project="server"', "Remote", `indent ${projectId === "server" ? "selected" : ""}`, "▣")}<div class="section-label">CHAT SPACES · NOT REPOSITORIES</div>${navRow("Research notes", 'data-project="scratch"', "Scratch", projectId === "scratch" ? "selected" : "", "◌")}<div class="inline-nav-actions"><button data-preview="Project collection import">Import collection…</button><button data-preview="Recoverable generated-folder cleanup">Cleanup…</button></div>`;
  } else if (navigatorMode === "files") {
    content = `<input type="search" id="file-filter" aria-label="Filter sample project files" placeholder="Filter project files…" /><div class="section-label">${escapeHtml(project().name.toUpperCase())} · FILES</div><div class="file-entry">${navRow("apps / desktop / src", 'data-preview="File and folder operations"', "", "", "▾")}</div><div class="file-entry" data-file-name="workbench.rs">${navRow("workbench.rs", 'data-route="editor"', "M", "deep", "◇")}</div><div class="file-entry" data-file-name="app.rs">${navRow("app.rs", 'data-route="editor"', "", "deep", "◇")}</div><div class="file-entry" data-file-name="theme.rs">${navRow("crates/ui/src/theme.rs", 'data-route="editor"', "M", "indent", "◇")}</div><div class="file-entry" data-file-name="workspace-notes.md">${navRow("workspace-notes.md", 'data-route="markdown"', "", "", "▤")}</div><div class="inline-nav-actions"><button data-preview="Create, rename, move and safely delete files">File actions…</button><button data-route="search">Search project</button><button data-preview="Image and PDF previews">Previews…</button></div>`;
  } else {
    content =
      `<div class="button-row"><button id="navigator-new-session" class="primary-button">＋ Conversation</button><button data-preview="Supported history import">Import…</button></div><div class="section-label">${escapeHtml(project().name.toUpperCase())} · SAVED HISTORY</div>` +
      project()
        .sessions.map((item, index) =>
          navRow(
            item.title,
            `data-session="${index}"`,
            item.empty ? "Draft" : "Sample",
            index === project().selected ? "selected" : "",
            "◌",
          ),
        )
        .join("") +
      `<div class="inline-nav-actions"><button data-project="scratch">Standalone chat space</button><button data-preview="Session scopes and retained history">History scope…</button></div>`;
  }
  $("#navigator-content").innerHTML = content;
  $("#navigator-content").setAttribute(
    "aria-labelledby",
    `nav-${navigatorMode}`,
  );
}
function selectProject(id) {
  projectId = id;
  editCode = false;
  $("#project-name").textContent = project().name;
  $("#host-name").textContent = project().host;
  $("#branch-name").textContent =
    id === "scratch" ? "No Git repository" : "design/native-workspace";
  $("#scope-status").textContent =
    `${project().host === "This Mac" ? "Local" : "Relay"} · ${project().name} · ${project().path}`;
  $("#session-scope").textContent =
    `${project().name} · ${project().host} · Sample session`;
  renderNavigator();
  renderPrimary();
  renderAgent();
  renderDock();
  if (narrow.matches) setSidebar(false);
  announce(
    `${project().name}, ${project().host}. Existing conversations are not moved between roots.`,
  );
}
function selectSession(index) {
  project().selected = index;
  lens = "chat";
  showAgent();
  renderNavigator();
  renderAgent();
  if (narrow.matches) setSidebar(false);
  $("#prompt").focus();
}
function newConversation() {
  project().sessions.push(makeSession("New conversation", true));
  selectSession(project().sessions.length - 1);
}
function showAgent() {
  workspace.dataset.split = "true";
  workspace.dataset.mobilePane = "agent";
  $("#toggle-split").setAttribute("aria-pressed", "true");
}
function renderDocumentTabs() {
  $("#document-tabs").innerHTML = openedDocuments
    .map(
      (id) =>
        `<button role="tab" id="document-${id}" aria-controls="primary-content" aria-selected="${id === selectedDocument}" tabindex="${id === selectedDocument ? "0" : "-1"}" data-document="${id}">${id === "editor" ? "◇" : id === "markdown" ? "▤" : "◫"} ${escapeHtml(titles[id])}${id === "editor" && project().code !== initialCode ? " •" : ""}</button>`,
    )
    .join("");
  $("#primary-content").setAttribute(
    "aria-labelledby",
    `document-${selectedDocument}`,
  );
  $$(".project-tools [data-route]").forEach((button) => {
    button.dataset.active = String(button.dataset.route === selectedDocument);
  });
}
function openDocument(id) {
  if (!openedDocuments.includes(id)) openedDocuments.push(id);
  selectedDocument = id;
  workspace.dataset.mobilePane = "primary";
  renderDocumentTabs();
  renderPrimary();
  if (narrow.matches) setSidebar(false);
  announce(`${titles[id]} document`);
}
function capabilityCards(route) {
  return capabilities
    .filter((item) => item.route === route)
    .map(
      (item) =>
        `<section class="capability-card"><button data-capability="${item.id}">${escapeHtml(item.label)} ↗</button><p>${escapeHtml(item.boundary)}</p></section>`,
    )
    .join("");
}
function documentPage(title, lead, content) {
  return `<article class="document-page"><div class="eyebrow">${escapeHtml(project().name.toUpperCase())} · ${escapeHtml(project().host.toUpperCase())} · DESIGN DATA</div><h1>${title}</h1><p class="page-lead">${lead}</p>${content}</article>`;
}
function subtabs(group, active, labels) {
  return `<div class="document-subtabs">${Object.entries(labels)
    .map(
      ([id, title]) =>
        `<button data-${group}="${id}" class="${active === id ? "selected" : ""}" aria-pressed="${active === id}">${title}</button>`,
    )
    .join("")}</div>`;
}
function renderPrimary() {
  renderDocumentTabs();
  const reviewed = Boolean(project().reviewed);
  const demoProposal = project().proposal || "Pending review";
  const target = $("#primary-content");
  const scope = `<div class="scope-strip">▣ ${escapeHtml(project().host)} <span>·</span> ${escapeHtml(project().path)}</div>`;
  if (selectedDocument === "editor") {
    target.innerHTML = `<div class="editor-breadcrumb"><span>apps › desktop › src › workbench.rs</span><button id="edit-buffer">${editCode ? "Read preview" : "Edit demo buffer"}</button></div>${editCode ? '<textarea class="buffer-editor" id="code-buffer" aria-label="Illustrative code buffer"></textarea>' : '<pre class="code-editor" id="code-preview" aria-label="Illustrative Rust source" tabindex="0"></pre>'}<p class="code-note">Rust · UTF-8 · Sample buffer · ${project().code !== initialCode ? "Unsaved demo edits retained in memory" : "No file is read or written"}. Independent from the conversation pane.</p>`;
    if (editCode) $("#code-buffer").value = project().code;
    else
      project()
        .code.split("\n")
        .forEach((line) => {
          const row = document.createElement("span");
          row.className = "code-line";
          const escaped = escapeHtml(line || " ");
          row.innerHTML = line.trim().startsWith("//")
            ? `<span class="code-comment">${escaped}</span>`
            : escaped.replace(
                /\b(pub|struct|impl|fn|use|self)\b/g,
                '<span class="code-keyword">$1</span>',
              );
          $("#code-preview").append(row);
        });
    return;
  }
  if (selectedDocument === "markdown") {
    target.innerHTML = documentPage(
      "Workspace notes",
      "Visual and raw are two views of the same document, not separate saved files.",
      `<div class="button-row"><button id="toggle-markdown">${rawMarkdown ? "Visual preview" : "Raw Markdown"}</button><button data-preview="Save a dirty Markdown document">Save…</button></div>${rawMarkdown ? '<textarea id="markdown-buffer" class="buffer-editor" aria-label="Sample Markdown buffer"></textarea>' : '<div class="markdown-preview" id="markdown-preview"></div>'}`,
    );
    if (rawMarkdown) $("#markdown-buffer").value = project().markdown;
    else
      project()
        .markdown.split("\n")
        .filter(Boolean)
        .forEach((line) => {
          const item = document.createElement(
            line.startsWith("# ") ? "h2" : "p",
          );
          item.textContent = line.replace(/^# /, "");
          $("#markdown-preview").append(item);
        });
    return;
  }
  if (selectedDocument === "overview")
    target.innerHTML = documentPage(
      "Your workspace, in context",
      "A landing place for recorded activity and recent projects — not a replacement for the editor.",
      `${scope}<div class="metric-grid"><div class="metric"><span>PROJECTS · SAMPLE</span><strong>3</strong><span>Local and relay scopes</span></div><div class="metric"><span>CHAT SPACES</span><strong>1</strong><span>Separate history</span></div><div class="metric"><span>UNCONFIRMED USAGE</span><strong>—</strong><span>Unknown, not zero</span></div></div><div class="button-row"><button data-route="editor">Resume editor</button><button data-route="sessions">Conversation history</button><button data-preview="Project collection import">Import project…</button></div>${capabilityCards("overview")}`,
    );
  else if (selectedDocument === "git") renderGit(target, scope);
  else if (selectedDocument === "knowledge")
    target.innerHTML = documentPage(
      "Knowledge",
      "File-first objects, relationships and human-reviewed proposals remain a full workspace.",
      `${scope}${subtabs("knowledge-tab", knowledgeTab, { objects: "Objects", relations: "Relations", proposals: "Proposals · 2" })}${knowledgeTab === "objects" ? '<table class="data-table"><tr><th>Kind / title</th><th>State</th></tr><tr><td><button data-preview="Intent object: native workspace">◇ Intent · Native workspace</button><small>intent/workspace.md · canonical file</small></td><td>In review</td></tr><tr><td><button data-preview="Concept object: execution scope">◇ Concept · Execution scope</button><small>concepts/execution-scope.md</small></td><td>Accepted</td></tr><tr><td><button data-preview="Decision object: preserve the workbench">◇ Decision · Preserve the workbench</button><small>decisions/workbench.md</small></td><td>Draft</td></tr></table>' : knowledgeTab === "relations" ? '<div class="graph-flow"><button class="graph-node" data-preview="Knowledge relation evidence">Intent: native workspace<small>implements ↓ · derived relation, not proof</small></button><span class="graph-connector">↓</span><button class="graph-node" data-preview="Knowledge relation evidence">Decision: preserve every capability</button></div>' : `<div class="sample-banner">Proposed write · decisions/workbench.md<br/>“Retain both document panes and all project tools.”</div><p class="muted" id="proposal-state">${demoProposal}</p><div class="button-row"><button id="accept-proposal">Accept demo proposal</button><button id="reject-proposal">Reject demo proposal</button></div><p class="muted">Changes fixture state only. No project file is written.</p>`}${capabilityCards("knowledge")}`,
    );
  else if (selectedDocument === "spec")
    target.innerHTML = documentPage(
      "Intent & Specifications",
      "The phase cockpit, governing documents, tasks, gates and traceability are not reduced to a chat sidebar.",
      `${scope}${subtabs("spec-tab", specTab, { workflow: "Workflow", intents: "Intent inbox", trace: "Traceability", contract: "Agent contract" })}<div class="phase-strip"><span>Intent</span><span>Specify</span><span>Clarify</span><span>Plan</span><span class="current">Tasks</span><span>Implement</span><span>Converge</span></div>${specTab === "trace" ? '<table class="data-table"><tr><th>Requirement</th><th>Task / evidence</th></tr><tr><td>FR-01 · Preserve panes</td><td>T001 → worktree/ui-materials<small>HEAD evidence requires fresh review</small></td></tr><tr><td>FR-02 · Native material</td><td>T002 → no linked result<small>Unknown, not complete</small></td></tr></table>' : specTab === "contract" ? '<div class="sample-banner">Agent contract · hash-bound decisions<br/>Permission ceiling: ask before mutation. Heuristic audit is not a sandbox or proof of compliance.</div><div class="button-row"><button data-preview="Review the exact governing document revision">Review document…</button></div>' : '<table class="data-table"><tr><th>Task / intent</th><th>Evidence</th></tr><tr><td>T001 · Preserve document state<small>Open reviewer checklist · blocks implementation gate</small></td><td><button data-preview="Task worktree creation gate">Worktree…</button></td></tr><tr><td>T002 · Native material fallback<small>Missing convergence evidence</small></td><td><button data-route="checks">Checks</button></td></tr></table>'}<div class="button-row"><button data-preview="Capture intent from the current composer">Capture intent…</button><button data-route="markdown">Open governing document</button><button data-preview="Spec next-step gate">Review next step…</button></div>${capabilityCards("spec")}`,
    );
  else if (selectedDocument === "artifacts")
    target.innerHTML = documentPage(
      "Artifacts & Evidence",
      "Content-addressed captures are distinct from project files and from a claim that the evidence is correct.",
      `${scope}<div class="button-row"><button data-preview="Capture clipboard as an artifact">Capture clipboard…</button><button data-preview="Capture the current file as an artifact">Capture file…</button></div><table class="data-table"><tr><th>Artifact · sample</th><th>Scope</th></tr><tr><td><button data-preview="Bounded artifact preview">workspace-review.md</button><small>Text · content hash · provenance retained</small></td><td>Local</td></tr><tr><td><button data-preview="Expiring relay artifact sharing and revocation">ui-evidence.png</button><small>Sharing requires remote storage and explicit consent</small></td><td>Not shared</td></tr></table>${capabilityCards("artifacts")}`,
    );
  else if (selectedDocument === "security")
    target.innerHTML = documentPage(
      "Security Audits",
      "Findings, coverage and triage — not a green security badge or an implicit scanner run.",
      `${scope}<div class="scope-strip">Fixture report · unverified findings · coverage incomplete</div><table class="data-table"><tr><th>Finding / evidence</th><th>Triage</th></tr><tr><td>Sample: unbounded input<small>Illustrative finding, not a vulnerability report about this repository</small></td><td><select id="triage" aria-label="Sample finding triage"><option>Open</option><option>Accepted</option><option>False positive</option><option>Fixed</option></select></td></tr></table><div class="button-row"><button data-preview="Start a reviewed security audit conversation">Start audit in chat…</button><button data-preview="Explicit sandboxed local check">Sandboxed check…</button><button data-preview="Export original findings and local triage">Export evidence…</button></div>${capabilityCards("security")}`,
    );
  else if (selectedDocument === "review")
    target.innerHTML = documentPage(
      "Task Review & Checkpoints",
      "A saved-file baseline and selected evidence, not an automatic attribution of all dirty files to an agent.",
      `${scope}<div class="sample-banner">Baseline: before workspace refresh · sample<br/>Unsaved editor buffers are outside checkpoint restoration.</div><pre class="diff-block"><span class="diff-line removed">- hide_advanced_tools();</span><span class="diff-line added">+ retain_all_capabilities();</span><span class="diff-line added">+ organize_by_scope();</span></pre><div class="button-row"><button id="mark-reviewed">${reviewed ? "Mark unreviewed" : "Mark reviewed"}</button><button data-preview="Capture a saved-file checkpoint">Capture baseline…</button><button data-preview="Fresh one-file restore confirmation">Restore selected file…</button></div><p id="review-progress" class="muted">${reviewed ? "1" : "0"} of 1 sample diffs reviewed</p>${capabilityCards("review")}`,
    );
  else if (selectedDocument === "compare")
    target.innerHTML = documentPage(
      "A/B Comparison",
      "A full comparison document with separate worktree and adapter approvals, not a toggle that launches two agents.",
      `${scope}<table class="data-table"><tr><th>Variant A</th><th>Variant B</th></tr><tr><td>Same approved input<small>Detached worktree · not created</small></td><td>Same approved input<small>Detached worktree · not created</small></td></tr><tr><td>Observed cost: unknown</td><td>Observed cost: unknown</td></tr></table><div class="button-row"><button data-preview="Prepare two comparison worktrees">Review worktree plan…</button><button data-preview="Authorize A/B adapters separately">Review agent starts…</button></div>${capabilityCards("compare")}`,
    );
  else if (selectedDocument === "impact")
    target.innerHTML = documentPage(
      "Rust Impact",
      "Bounded references, dependents and candidate tests. Not compiler-complete analysis or a proof of coverage.",
      `${scope}<table class="data-table"><tr><th>Symbol / candidate</th><th>Evidence</th></tr><tr><td>WorkspacePanes<small>Sample reference → workbench.rs</small></td><td>AST / lexical</td></tr><tr><td>pane_state_roundtrip<small>Candidate test, not proven coverage</small></td><td>Needs review</td></tr></table>${capabilityCards("impact")}`,
    );
  else if (selectedDocument === "servers")
    target.innerHTML = documentPage(
      "Servers & SSH",
      "Administration keeps the selected server's accounts, paths, grants and tools separate from this Mac.",
      '<table class="data-table"><tr><th>Host / scope</th><th>State</th></tr><tr><td>This Mac<small>Local agents, Keychain and projects</small></td><td>Demo</td></tr><tr><td>Studio relay<small>Remote harnesses, grants and /srv projects</small></td><td>Not connected</td></tr><tr><td>SSH: build-host<small>System OpenSSH · explicit host-key decision</small></td><td>Not connected</td></tr></table><div class="button-row"><button data-preview="One-use relay pairing">Pair server…</button><button data-preview="SSH host setup and relay bootstrap">Add SSH host…</button><button data-preview="Remote clone or reviewed snapshot copy">Clone / copy project…</button><button data-preview="Remote harness accounts and authorization">Manage host tools…</button></div>' +
        capabilityCards("servers"),
    );
  else if (selectedDocument === "search")
    target.innerHTML = documentPage(
      "Search this project",
      "Project file search stays scoped to the selected root. This is not global history FTS.",
      `${scope}<input id="project-query" type="search" aria-label="Search sample project" placeholder="Try: WorkspacePanes" /><div id="project-search-results" class="sample-banner">Enter a query to search the illustrative project index.</div>`,
    );
}
function renderGit(target, scope) {
  const staged = Boolean(project().staged);
  if (projectId === "scratch") {
    target.innerHTML = documentPage(
      "No repository in this chat space",
      "Scratch chats keep their own history. Git commands do not silently acquire a different repository.",
      '<div class="button-row"><button data-project="dunlin">Select repository project</button></div>',
    );
    return;
  }
  const tabs = subtabs("git-tab", gitTab, {
    changes: "Changes · 3",
    history: "Graph & history",
    worktrees: "Worktrees",
    state: "Stash & conflicts",
  });
  let content;
  if (gitTab === "changes")
    content = `<table class="data-table"><tr><th>Saved file · sample</th><th>Index</th></tr><tr><td><button data-preview="Read-only file diff">apps/desktop/src/workbench.rs</button><small>Modified · pre-existing changes are not agent attribution</small></td><td>${staged ? "Staged (demo)" : "Unstaged"}</td></tr><tr><td>crates/ui/src/theme.rs</td><td>Unstaged</td></tr><tr><td>workspace-notes.md</td><td>Untracked</td></tr></table><div class="button-row"><button id="stage-file">${staged ? "Unstage sample file" : "Stage sample file"}</button><button data-preview="Commit staged changes">Commit…</button><button data-preview="AI-assisted commit confirmation">AI commit…</button><button data-preview="Fresh pull or push plan">Pull / push…</button></div>`;
  else if (gitTab === "history")
    content =
      '<table class="data-table"><tr><th>Commit / graph</th><th>Reference</th></tr><tr><td>● Refine workspace state<small>Sample commit · explicit parent selection</small></td><td>HEAD</td></tr><tr><td>│ ● Preserve native controls</td><td>main</td></tr><tr><td>●─╯ Share document buffers</td><td>base</td></tr></table><div class="button-row"><button data-preview="Create a branch without checking it out">New branch…</button><button data-preview="Switch branch with dirty-buffer guards">Switch branch…</button><button data-preview="Bounded commit patch and parent selection">Inspect patch…</button></div>';
  else if (gitTab === "worktrees")
    content =
      '<table class="data-table"><tr><th>Worktree / base</th><th>State</th></tr><tr><td>ui-materials<small>dunlin/subtask/ui-materials · sample base</small></td><td>Not opened</td></tr></table><div class="button-row"><button data-preview="Create one registered subtask worktree">Create subtask worktree…</button><button data-preview="Open a worktree as a separate project">Open as project…</button><button data-preview="Guarded clean-worktree removal">Remove…</button></div>';
  else
    content =
      '<div class="sample-banner">Stash, conflicts and sequencer state are separate from a clean/dirty summary. Unknown remote state is not a clean repository.</div><div class="button-row"><button data-preview="Stash operation with fresh repository state">Stash…</button><button data-preview="Merge, rebase and cherry-pick plan">Merge / rebase / cherry-pick…</button><button data-preview="Conflict resolution with saved-buffer guards">Resolve conflicts…</button></div>';
  target.innerHTML = documentPage(
    "Source Control",
    "The full repository workbench stays discoverable: changes, history, branches, worktrees and sequencer state.",
    `${scope}${tabs}${content}${capabilityCards("git")}`,
  );
}
function renderAgent() {
  $("#active-session-title").textContent = session().title;
  $(".agent-header strong").textContent = session().title;
  $("#mode-select").value = session().mode;
  $("#agent-select").value = session().agent;
  $$("[data-lens]").forEach((button) => {
    const active = button.dataset.lens === lens;
    button.setAttribute("aria-selected", String(active));
    button.tabIndex = active ? 0 : -1;
  });
  const target = $("#agent-content");
  target.replaceChildren();
  $("#composer").hidden = lens !== "chat";
  if (lens === "graph")
    target.innerHTML =
      '<div class="eyebrow">CONVERSATION GRAPH · FIXTURE</div><div class="graph-flow"><button class="graph-node" data-preview="Recorded prompt and execution scope">User request<small>Refresh presentation, preserve capabilities</small></button><span class="graph-connector">↓</span><button class="graph-node" data-preview="Observed tool call evidence">Read project context<small>Tool call, not a fabricated child agent</small></button><span class="graph-connector">↓</span><button class="graph-node" data-preview="Observed response and plan evidence">Agent response<small>Plan and source links</small></button></div><p class="muted">Graph remains an explicit conversation destination, not hidden inside a generic Activity page.</p>';
  else if (lens === "trace")
    target.innerHTML =
      '<div class="eyebrow">CONVERSATION TRACE · FIXTURE</div><h2>One request, inspectable spans</h2><div class="trace-row"><span>Request</span><div class="trace-track"><i class="trace-bar full"></i></div><span>2.4 s</span></div><div class="trace-row"><span>Read context</span><div class="trace-track"><i class="trace-bar short"></i></div><span>0.8 s</span></div><div class="trace-row"><span>Response</span><div class="trace-track"><i class="trace-bar medium"></i></div><span>1.6 s</span></div><div class="sample-banner">Durations are illustrative. Unreported tokens, provider prices and internal child work remain unknown.</div><button class="secondary-button" data-route="settings:usage">Account limits & billing evidence</button>';
  else {
    const append = (text, user = false) => {
      const row = document.createElement("div");
      row.className = `message ${user ? "user" : "agent"}`;
      row.textContent = text;
      target.append(row);
    };
    if (!session().empty) {
      append(
        "Refresh the interface without losing the IDE: projects and groups, both document panes, Git, terminal, Knowledge, Spec and every agent tool.",
        true,
      );
      const heading = document.createElement("div");
      heading.className = "agent-attribution";
      heading.innerHTML =
        "<strong>✧ OpenCode</strong><small>Sample conversation · not connected</small>";
      target.append(heading);
      append(
        "I’ll preserve the full workbench. The editor stays beside the agent; project workspaces remain visible, and execution tools share a dock instead of competing with documents.",
      );
      const details = document.createElement("details");
      details.className = "tool-card";
      details.innerHTML =
        "<summary>⌕ Read project context · 3 files</summary><p>Illustrative tool evidence. Exact paths, host and approval policy remain visible before real operations.</p>";
      target.append(details);
      const plan = document.createElement("div");
      plan.className = "plan-card";
      plan.innerHTML =
        '<header><strong>Implementation plan</strong><small>Design fixture</small></header><div class="plan-step"><span class="step-check">✓</span> Inventory existing capabilities</div><div class="plan-step"><span class="step-pending">◌</span> Refresh surfaces without changing scope</div><button class="inline-link" data-action="coverage">Inspect preservation map ↗</button>';
      target.append(plan);
    }
    session().messages.forEach((text) => {
      append(text, true);
      append(
        "Demo response only. No agent was called, no command was run and no project content was modified.",
      );
    });
  }
  $("#prompt").value = session().draft;
  $(".send-button").disabled = !session().draft.trim();
  target.setAttribute("aria-labelledby", `lens-${lens}`);
}
function renderDock() {
  const target = $("#dock-content");
  if (dockMode === "terminal")
    target.innerHTML = `<div class="terminal-toolbar"><button data-preview="Independent terminal tab">zsh · ${escapeHtml(project().name)} · demo</button><button data-preview="Create another local, relay or SSH terminal">＋ Session</button><button data-preview="Dock placement and resize">Dock position…</button><button id="split-terminal">${terminalSplit ? "Unsplit preview" : "Split terminal"}</button></div><div class="terminal-panes ${terminalSplit ? "split" : ""}"><div><pre class="terminal-output"><span class="terminal-prompt">${escapeHtml(project().host)} ~/ ${escapeHtml(project().name)} %</span> cargo check -p dunlin --locked
[preview] Command not executed. Exact target and argv require confirmation.</pre></div>${terminalSplit ? '<div><pre class="terminal-output">Second independent shell · illustrative pane\nNo PTY started. Existing terminal tabs stay alive when the dock changes.</pre></div>' : ""}</div>`;
  else if (dockMode === "run")
    target.innerHTML =
      '<div class="dock-card-row"><strong>Cargo · check</strong><small>cargo check -p dunlin --locked</small><button data-preview="Run exact manifest task">Review command…</button></div><div class="dock-card-row"><strong>Cargo · test</strong><small>Project environment names, not secret values</small><button data-preview="Run exact manifest task">Review command…</button></div>';
  else if (dockMode === "agents")
    target.innerHTML =
      '<div class="dock-card-row"><strong>Workspace refinement</strong><small>Sample observed run · report not verified</small><button data-preview="Project peer run, report and inbox">Report & inbox…</button></div><div class="dock-card-row"><strong>Deferred verification</strong><small>Waiting for exact dependencies · NOT approved</small><button data-preview="Approve one exact deferred turn">Review approval…</button></div>';
  else
    target.innerHTML =
      '<div class="dock-card-row"><strong>No fresh check report</strong><small>Unknown is not a pass · later edits invalidate old evidence</small><button data-preview="Ordinary workbench check: exact argv and cwd">Configure check…</button></div><div class="dock-card-row"><strong>Task review</strong><small>Checkpoints and saved-source evidence</small><button data-route="review">Open review</button></div>';
  target.setAttribute("aria-labelledby", `dock-${dockMode}`);
}
function renderInspector(kind = "context") {
  const isBudget = kind === "budget";
  $("#inspector-title").textContent = isBudget
    ? "Task Budget"
    : "Context & Decisions";
  $("#inspector-content").innerHTML =
    `<div class="eyebrow">CONVERSATION SCOPE</div><h2>${escapeHtml(project().name)}</h2><p>${escapeHtml(project().host)} · ${escapeHtml(project().path)}</p><dl><div><dt>Agent</dt><dd>OpenCode · demo</dd></div><div><dt>Mode</dt><dd>${escapeHtml($("#mode-select").value)}</dd></div><div><dt>Permissions</dt><dd>Ask first</dd></div><div><dt>Inheritance</dt><dd>Project → Personal group</dd></div></dl>${isBudget ? '<h3>Dispatch gates</h3><p>Requests/time and observed USD evidence. Not a guaranteed provider billing cap.</p><div class="button-row"><button data-preview="Task budget stop, release and forget">Configure limits…</button></div>' : '<h3>Client-controlled envelope</h3><p>Project roots, attached resources and effective defaults. Harness-owned history and system prompts may be unknown.</p><h3>Decision memory</h3><p>Explicit, revision-bound project decisions. Not credentials or automatic instructions.</p><div class="button-row"><button data-preview="Approve a revision-bound decision note">Review decisions…</button><button data-preview="MCP exclusions, conversation skills and attachments">Attachments / MCP…</button></div>'}${capabilityCards(kind)}`;
  $("#inspector-content dl > div:first-child dd").textContent =
    `${session().agent} · demo`;
  $("#inspector-content dl > div:last-child dd").textContent =
    projectId === "scratch"
      ? "Chat space · no repository group"
      : `Project → ${projectId === "server" ? "Studio" : "Personal"} group`;
  setInspector(true);
}
function showWorkflow(
  title,
  capability = null,
  opener = document.activeElement,
) {
  $("#workflow-title").textContent = title;
  const boundary =
    capability?.boundary ||
    "The native workflow keeps its existing validation and separate confirmation. This view only demonstrates where it belongs in the refreshed workbench.";
  const applicationSetting = Boolean(opener?.closest("#settings-dialog"));
  const targetHost = applicationSetting
    ? selectedSettings === "remote"
      ? "Named host — select explicitly"
      : "This Mac — application settings"
    : project().host;
  const targetProject = applicationSetting
    ? "Not a project override"
    : project().name;
  const targetRoot = applicationSetting
    ? "No repository root granted"
    : project().path;
  $("#workflow-content").innerHTML =
    `<p class="muted">${escapeHtml(boundary)}</p><div class="workflow-scope">Target: ${escapeHtml(targetHost)}\nProject: ${escapeHtml(targetProject)}\nRoot: ${escapeHtml(targetRoot)}\nMode: preview only — no operation authorized</div><ul><li>No process, model request, Git mutation, file write or credential access.</li><li>Local and remote scopes are never silently substituted.</li><li>In the native app the exact command, content or revision would be reviewed before a separate action.</li></ul>`;
  openDialog("#workflow-dialog", opener);
}
function openSettings(id = "appearance", opener = $("#open-settings")) {
  selectedSettings = id;
  $("#settings-search").value = "";
  renderSettings();
  openDialog("#settings-dialog", opener);
}
function renderSettings() {
  const query = $("#settings-search").value.toLowerCase();
  $("#settings-navigation").innerHTML = settings
    .filter((item) =>
      `${item.label} ${item.detail}`.toLowerCase().includes(query),
    )
    .map(
      (item) =>
        `<button data-settings="${item.id}" class="${selectedSettings === item.id ? "selected" : ""}" aria-current="${selectedSettings === item.id ? "page" : "false"}">${escapeHtml(item.label)}</button>`,
    )
    .join("");
  const item = settings.find((page) => page.id === selectedSettings);
  const actions = {
    general: [
      ["Startup & session restoration", "Select startup view", "Preview…"],
      [
        "Desktop notifications",
        "macOS consent remains explicit",
        "Review permission…",
      ],
    ],
    appearance: [
      [
        "Theme, material & motion",
        "Live design controls above the window; system Reduce Motion remains additive",
        "Appearance…",
      ],
      [
        "Interface font & density",
        "Reading zoom is separate from navigation density",
        "Preview…",
      ],
    ],
    workspace: [
      [
        "Project / group defaults",
        "Trust, inherited runtime values and source labels",
        "Open defaults…",
      ],
      [
        "Collections & watched folders",
        "Discovery is not automatic checkout",
        "Configure…",
      ],
    ],
    chat: [
      [
        "Conversation presentation",
        "Reasoning, defaults and reading",
        "Preview…",
      ],
      [
        "Prompt queue & notifications",
        "Queued does not mean dispatched",
        "Preview…",
      ],
    ],
    editor: [
      [
        "Editor and previews",
        "Font, minimap, supported previews and zoom",
        "Configure…",
      ],
      [
        "Terminal shell integration",
        "New terminals only; no dotfile rewrite",
        "Preview…",
      ],
    ],
    agents: [
      [
        "OpenCode / Claude Code / Codex",
        "Verified readiness, accounts, default and visibility are separate",
        "Test / setup…",
      ],
      [
        "History import",
        "Read-only supported stores; never a paid agent call",
        "Import…",
      ],
    ],
    skills: [
      [
        "Installed & available skills",
        "Shared discovery, companions and ownership-checked updates",
        "Manage…",
      ],
      [
        "Project versus conversation scope",
        "Attaching instructions does not install scripts",
        "Review scope…",
      ],
    ],
    extensions: [
      ["WASM governance pack", "Install and enable are separate", "Manage…"],
      [
        "Spec & intent tools",
        "Disable contributions without deleting documents",
        "Configure…",
      ],
    ],
    permissions: [
      [
        "Agent approval policy",
        "Permission ceiling and execution boundaries",
        "Review…",
      ],
      [
        "macOS privacy permissions",
        "Accessibility, notification and other TCC statuses",
        "Permission center…",
      ],
    ],
    credentials: [
      [
        "Stored credentials",
        "This Mac's Keychain, never a remote host's values",
        "Manage metadata…",
      ],
      [
        "Use-only access policies",
        "HTTP/CLI recipes, expiry, approval and activity",
        "Review policy…",
      ],
    ],
    security: [
      [
        "Audit configuration",
        "Supported tool visibility and boundaries",
        "Configure…",
      ],
      [
        "Project audits",
        "Explicit source-only conversation or sandboxed checks",
        "Review workflow…",
      ],
    ],
    remote: [
      [
        "Named relay / SSH hosts",
        "Remote tools, accounts, grants and pairing",
        "Manage…",
      ],
      [
        "Project clone or snapshot copy",
        "Explicit operation, not synchronization",
        "Review target…",
      ],
    ],
    usage: [
      [
        "Limits & alerts",
        "Account allowances, missing data and alert thresholds",
        "Configure…",
      ],
      [
        "Cost evidence & attribution",
        "Main, subagent, internal and unknown; explicit aggregate sync",
        "Inspect report…",
      ],
    ],
    data: [
      [
        "Portable settings / project backup",
        "Exports exclude histories and credentials",
        "Export / import…",
      ],
      [
        "Recovery & factory reset",
        "Explicit confirmation and failure-safe data ownership",
        "Review…",
      ],
    ],
    diagnostics: [
      [
        "Agent logs & environment",
        "Redacted exports and bounded diagnostics",
        "Export…",
      ],
      ["Dunlin", "Version, local build and notices", "About…"],
    ],
  };
  $("#settings-content").innerHTML =
    `<h3>${escapeHtml(item.label)}</h3><p class="muted">${escapeHtml(item.detail)}</p>${actions[item.id].map(([label, detail, action]) => `<div class="setting-row"><span><strong>${label}</strong><small>${detail}</small></span><button data-preview="${escapeHtml(`${item.label}: ${label}`)}">${action}</button></div>`).join("")}<div class="sample-banner">Existing functional page retained. This is a presentation preview, not the native configuration form or a request for secrets.</div>`;
}
function renderCoverage() {
  const query = $("#coverage-search").value.toLowerCase();
  $("#coverage-summary").textContent =
    `${capabilities.length} existing capability families mapped to a visible workbench, navigator, dock or settings route. All 15 settings pages retained. A map is not a claim that a native redesign has been implemented.`;
  $("#coverage-list").innerHTML = capabilities
    .filter((item) =>
      `${item.family} ${item.label} ${item.boundary}`
        .toLowerCase()
        .includes(query),
    )
    .map(
      (item) =>
        `<section class="coverage-row" data-coverage-id="${item.id}"><span>${escapeHtml(item.family)}</span><div><strong>${escapeHtml(item.label)}</strong><p>${escapeHtml(item.boundary)}</p><a href="../../user-guide/${item.guide}.md">Current functionality reference ↗</a></div><button data-route="${item.route}">Open ↗</button></section>`,
    )
    .join("");
}
function openCoverage(opener = $("#show-coverage")) {
  $("#coverage-search").value = "";
  renderCoverage();
  openDialog("#coverage-dialog", opener);
}
function commandItems() {
  return [
    ...Object.keys(titles).map((route) => ({
      label: titles[route],
      family: "Document",
      route,
    })),
    ...capabilities.map((item) => ({
      label: item.label,
      family: item.family,
      route: item.route,
    })),
    ...Object.entries(projects).map(([id, item]) => ({
      label: item.name,
      family: `Project · ${item.host}`,
      project: id,
    })),
    ...project().sessions.map((item, index) => ({
      label: item.title,
      family: "Conversation",
      session: index,
    })),
  ];
}
function renderCommands() {
  const query = $("#command-query").value.toLowerCase();
  const items = commandItems().filter((item) =>
    `${item.label} ${item.family}`.toLowerCase().includes(query),
  );
  $("#command-results").innerHTML = items.length
    ? items
        .map(
          (item, index) =>
            `<button data-command-index="${index}"><span>${escapeHtml(item.label)}</span><small>${escapeHtml(item.family)}</small></button>`,
        )
        .join("")
    : '<p class="muted">No matching file, session, project or tool.</p>';
  $$("[data-command-index]").forEach((button) =>
    button.addEventListener("click", () => {
      const item = items[Number(button.dataset.commandIndex)];
      closeDialogs();
      if (item.route) navigate(item.route);
      else if (item.project) selectProject(item.project);
      else selectSession(item.session);
    }),
  );
}
function openCommands(opener = document.activeElement) {
  $("#command-query").value = "";
  renderCommands();
  openDialog("#command-dialog", opener);
  $("#command-query").focus();
}
function navigate(route, opener = document.activeElement) {
  closeDialogs();
  if (route.startsWith("settings:")) {
    openSettings(route.split(":")[1], $("#open-commands"));
    return;
  }
  if (["projects", "files", "sessions"].includes(route)) {
    navigatorMode = route;
    renderNavigator();
    setSidebar(true);
    if (route === "files") openDocument("editor");
    return;
  }
  if (["chat", "graph", "trace"].includes(route)) {
    lens = route;
    showAgent();
    renderAgent();
    if (narrow.matches) setSidebar(false);
    return;
  }
  if (["terminal", "run", "agents", "checks"].includes(route)) {
    setDock(route);
    if (narrow.matches) setSidebar(false);
    return;
  }
  if (["context", "budget"].includes(route)) {
    renderInspector(route);
    return;
  }
  if (titles[route]) openDocument(route);
}

document.addEventListener("click", (event) => {
  const target = event.target.closest("button");
  if (!target) return;
  if (target.hasAttribute("data-close-dialog")) {
    target.closest("dialog").close();
    return;
  }
  if (target.dataset.route) navigate(target.dataset.route, target);
  else if (target.dataset.project) {
    closeDialogs();
    selectProject(target.dataset.project);
  } else if (target.dataset.session !== undefined)
    selectSession(Number(target.dataset.session));
  else if (target.dataset.document) openDocument(target.dataset.document);
  else if (target.dataset.nav) {
    navigatorMode = target.dataset.nav;
    renderNavigator();
  } else if (target.dataset.lens) {
    lens = target.dataset.lens;
    renderAgent();
  } else if (target.dataset.dock) setDock(target.dataset.dock);
  else if (target.dataset.settings) {
    selectedSettings = target.dataset.settings;
    renderSettings();
  } else if (target.dataset.preview)
    showWorkflow(target.dataset.preview, null, target);
  else if (target.dataset.capability) {
    const item = capabilities.find(
      (item) => item.id === target.dataset.capability,
    );
    showWorkflow(item.label, item, target);
  } else if (target.dataset.gitTab) {
    gitTab = target.dataset.gitTab;
    renderPrimary();
  } else if (target.dataset.knowledgeTab) {
    knowledgeTab = target.dataset.knowledgeTab;
    renderPrimary();
  } else if (target.dataset.specTab) {
    specTab = target.dataset.specTab;
    renderPrimary();
  } else if (target.dataset.action === "coverage") openCoverage(target);
  else if (target.id === "navigator-new-session") newConversation();
  else if (target.id === "edit-buffer") {
    editCode = !editCode;
    renderPrimary();
  } else if (target.id === "toggle-markdown") {
    rawMarkdown = !rawMarkdown;
    renderPrimary();
  } else if (target.id === "stage-file") {
    project().staged = !project().staged;
    renderPrimary();
    announce(
      "Only illustrative staging state changed. No Git command executed.",
    );
  } else if (target.id === "mark-reviewed") {
    project().reviewed = !project().reviewed;
    renderPrimary();
  } else if (["accept-proposal", "reject-proposal"].includes(target.id)) {
    project().proposal =
      target.id === "accept-proposal"
        ? "Accepted in demo only · no file written"
        : "Rejected in demo only";
    renderPrimary();
  } else if (target.id === "split-terminal") {
    terminalSplit = !terminalSplit;
    renderDock();
  }
});
document.addEventListener("input", (event) => {
  if (event.target.id === "code-buffer") {
    project().code = event.target.value;
    renderDocumentTabs();
  }
  if (event.target.id === "markdown-buffer")
    project().markdown = event.target.value;
  if (event.target.id === "file-filter")
    $$("[data-file-name]").forEach((row) => {
      row.hidden = !row.dataset.fileName.includes(
        event.target.value.toLowerCase(),
      );
    });
  if (event.target.id === "project-query") {
    $("#project-search-results").textContent = event.target.value.trim()
      ? "Sample match: apps/desktop/src/workbench.rs · WorkspacePanes (illustrative index, no disk scan)"
      : "Enter a query to search the illustrative project index.";
  }
});
$("#theme").addEventListener("change", setTheme);
dark.addEventListener("change", setTheme);
$("#glass").addEventListener("change", (event) => {
  root.dataset.glass = String(event.target.checked);
});
$("#reduce-motion").addEventListener("change", (event) => {
  root.dataset.motion = String(!event.target.checked);
});
$("#toggle-sidebar").addEventListener("click", () =>
  setSidebar(workspace.dataset.sidebar !== "true"),
);
$("#toggle-inspector").addEventListener("click", () => {
  if (workspace.dataset.inspector === "true") setInspector(false);
  else renderInspector();
});
$("#close-inspector").addEventListener("click", () => setInspector(false));
$("#toggle-split").addEventListener("click", () => {
  const split = workspace.dataset.split !== "true";
  workspace.dataset.split = String(split);
  workspace.dataset.mobilePane = split ? "agent" : "primary";
  $("#toggle-split").setAttribute("aria-pressed", String(split));
});
const focusAgent = document.createElement("button");
focusAgent.id = "focus-agent";
focusAgent.className = "focus-agent";
focusAgent.textContent = "Agent";
focusAgent.addEventListener("click", showAgent);
$(".pane-actions").prepend(focusAgent);
$("#toggle-dock").addEventListener("click", () => {
  const visible = workspace.dataset.dock !== "true";
  workspace.dataset.dock = String(visible);
  $("#toggle-dock").setAttribute("aria-expanded", String(visible));
});
$("#close-dock").addEventListener("click", () => {
  workspace.dataset.dock = "false";
  $("#toggle-dock").setAttribute("aria-expanded", "false");
  $("#toggle-dock").focus();
});
$("#choose-project").addEventListener("click", (event) => {
  $("#project-options").innerHTML = Object.entries(projects)
    .map(([id, item]) =>
      navRow(
        item.name,
        `data-project="${id}"`,
        item.host,
        id === projectId ? "selected" : "",
        "▣",
      ),
    )
    .join("");
  openDialog("#project-dialog", event.currentTarget);
});
$("#choose-host").addEventListener("click", (event) => {
  showWorkflow(
    "Execution environments — not a silent runtime retarget",
    null,
    event.currentTarget,
  );
  $("#workflow-content").insertAdjacentHTML(
    "beforeend",
    '<div class="button-row"><button data-route="servers">Manage named hosts</button><button data-project="server">Select remote API Service project</button></div>',
  );
});
$("#add-project").addEventListener("click", (event) =>
  showWorkflow(
    "Add a local project, import a collection or clone explicitly",
    null,
    event.currentTarget,
  ),
);
$("#new-document").addEventListener("click", (event) => {
  showWorkflow(
    "New document, conversation or terminal",
    null,
    event.currentTarget,
  );
  $("#workflow-content").insertAdjacentHTML(
    "beforeend",
    '<div class="button-row"><button data-route="markdown">Document</button><button id="dialog-new-session">Conversation</button><button data-route="terminal">Terminal</button></div>',
  );
});
document.addEventListener("click", (event) => {
  if (event.target.id === "dialog-new-session") {
    closeDialogs();
    newConversation();
  }
});
$("#new-conversation").addEventListener("click", newConversation);
$("#active-session-title").addEventListener("click", () =>
  navigate("sessions"),
);
$("#open-settings").addEventListener("click", (event) =>
  openSettings("appearance", event.currentTarget),
);
$("#show-coverage").addEventListener("click", (event) =>
  openCoverage(event.currentTarget),
);
$("#open-commands").addEventListener("click", (event) =>
  openCommands(event.currentTarget),
);
$("#attach-context").addEventListener("click", (event) =>
  showWorkflow(
    "Attachments, conversation skills & advertised MCP servers",
    capabilities.find((item) => item.id === "mcp"),
    event.currentTarget,
  ),
);
$("#settings-search").addEventListener("input", renderSettings);
$("#coverage-search").addEventListener("input", renderCoverage);
$("#command-query").addEventListener("input", renderCommands);
$("#command-query").addEventListener("keydown", (event) => {
  if (event.key === "ArrowDown") {
    event.preventDefault();
    $("#command-results button")?.focus();
  }
});
$("#prompt").addEventListener("input", () => {
  session().draft = $("#prompt").value;
  $(".send-button").disabled = !session().draft.trim();
});
$("#mode-select").addEventListener("change", (event) => {
  session().mode = event.target.value;
});
$("#agent-select").addEventListener("change", (event) => {
  session().agent = event.target.value;
});
$("#composer").addEventListener("submit", (event) => {
  event.preventDefault();
  const text = session().draft.trim();
  if (!text) return;
  session().messages.push(text);
  session().draft = "";
  renderAgent();
  $("#agent-content").scrollTop = $("#agent-content").scrollHeight;
  announce("Demo message only. No agent call or file write.");
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && $("dialog[open]")) {
    // A search input can consume Escape to clear its value in Chromium. Close
    // the top dialog consistently instead, including nested workflow previews.
    event.preventDefault();
    $$("dialog[open]").at(-1).close();
    return;
  }
  if (
    (event.metaKey || event.ctrlKey) &&
    event.key.toLowerCase() === "k" &&
    !$("dialog[open]")
  ) {
    event.preventDefault();
    openCommands();
  }
  if (
    (event.metaKey || event.ctrlKey) &&
    event.key === "Enter" &&
    document.activeElement === $("#prompt")
  ) {
    event.preventDefault();
    $("#composer").requestSubmit();
  }
  if (event.key === "Escape" && !$("dialog[open]")) {
    if (workspace.dataset.inspector === "true") setInspector(false);
    else if (narrow.matches && workspace.dataset.sidebar === "true")
      setSidebar(false);
  }
  const list = event.target.closest('[role="tablist"]');
  if (!list || event.target.getAttribute("role") !== "tab") return;
  const tabs = $$('[role="tab"]', list);
  const index = tabs.indexOf(event.target);
  let next;
  if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
  else if (event.key === "ArrowLeft")
    next = (index + tabs.length - 1) % tabs.length;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = tabs.length - 1;
  else return;
  event.preventDefault();
  const id = tabs[next].id;
  tabs[next].click();
  document.getElementById(id)?.focus();
});
function adaptLayout() {
  if (narrow.matches) setSidebar(false);
  syncLayout();
}
narrow.addEventListener("change", adaptLayout);
inspectorSheet.addEventListener("change", syncLayout);
[
  ["nav", "navigator-content"],
  ["lens", "agent-content"],
  ["dock", "dock-content"],
].forEach(([attribute, panel]) => {
  $$(`[data-${attribute}]`).forEach((button) => {
    button.id = `${attribute}-${button.dataset[attribute]}`;
    button.setAttribute("aria-controls", panel);
  });
});
$("#primary-content").setAttribute("role", "tabpanel");
$("#agent-content").setAttribute("role", "tabpanel");
$("#show-coverage").textContent = `Capabilities · ${capabilities.length}`;
renderNavigator();
renderPrimary();
renderAgent();
renderDock();
setTheme();
adaptLayout();
