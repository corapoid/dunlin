"use strict";

// Deliberately inert: no storage, network, agent, clipboard or filesystem access.
const $ = (selector, parent = document) => parent.querySelector(selector);
const $$ = (selector, parent = document) => [
  ...parent.querySelectorAll(selector),
];
const root = document.documentElement;
const workspace = $("#workspace");
const narrow = matchMedia("(max-width: 860px)");
const compact = matchMedia("(max-width: 1200px)");
const dark = matchMedia("(prefers-color-scheme: dark)");
const projects = {
  dunlin: {
    name: "Dunlin",
    sessions: [
      {
        title: "Refine the workspace",
        detail: "Navigation, with a little breathing room",
        draft: "",
        messages: [],
      },
      {
        title: "A better first five minutes",
        detail: "Onboarding that gets out of the way",
        draft: "",
        messages: [],
      },
      {
        title: "Review the release notes",
        detail: "Earlier today",
        draft: "",
        messages: [],
      },
    ],
    selected: 0,
  },
  relay: {
    name: "Relay",
    sessions: [
      {
        title: "Make connections clearer",
        detail: "Local and remote, never ambiguous",
        draft: "",
        messages: [],
      },
      {
        title: "A calmer connection flow",
        detail: "Pairing with explicit consent",
        draft: "",
        messages: [],
      },
      {
        title: "Check the host summary",
        detail: "Earlier today",
        draft: "",
        messages: [],
      },
    ],
    selected: 0,
  },
};
const files = [
  {
    name: "workspace.rs",
    lines: [
      "  impl Workspace {",
      "-     show_all_panels();",
      "+     show_current_document();",
      "+     inspector.reveal_on_request();",
      "  }",
      "",
      "  // Give the work a little room.",
    ],
  },
  {
    name: "palette.rs",
    lines: [
      "  struct Surfaces {",
      "-     document: translucent(),",
      "+     document: opaque(),",
      "+     navigation: system_material(),",
      "  }",
      "",
      "  // Glass frames content; it does not obscure it.",
    ],
  },
  {
    name: "navigation.rs",
    lines: [
      "  enum Destination {",
      "+     Work,",
      "+     Changes,",
      "+     Activity,",
      "  }",
      "",
      "  // Details belong to their context.",
    ],
  },
];
let projectId = "dunlin";
let selectedFile = 0;
const reviewed = new Set();
const currentProject = () => projects[projectId];
const currentSession = () =>
  currentProject().sessions[currentProject().selected];
const icon = (name) => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
};

function announce(message) {
  $("#announcement").textContent = message;
}

function openDialog(id, opener = document.activeElement) {
  const dialog = $(id);
  if (!dialog.open) {
    // Safari does not focus buttons on pointer activation. Establish the return
    // target explicitly before letting the native dialog manage modal focus.
    opener?.focus({ preventScroll: true });
    dialog.showModal();
  }
}

function setTheme() {
  const value = $("#theme").value;
  root.dataset.theme =
    value === "system" ? (dark.matches ? "dark" : "light") : value;
}

function syncNavigator() {
  const visible =
    workspace.dataset.sidebar === "true" &&
    !(compact.matches && workspace.dataset.inspector === "true");
  $("#navigator").inert = !visible;
  $("#navigator").setAttribute("aria-hidden", String(!visible));
  $("#toggle-sidebar").setAttribute("aria-expanded", String(visible));
  const covered =
    narrow.matches && (visible || workspace.dataset.inspector === "true");
  $(".work-area").inert = covered;
  $(".work-area").setAttribute("aria-hidden", String(covered));
}

function setSidebar(visible) {
  if (visible && compact.matches) setInspector(false, false);
  workspace.dataset.sidebar = String(visible);
  syncNavigator();
  if (visible && narrow.matches) $("#choose-project").focus();
  if (!visible && $("#navigator").contains(document.activeElement))
    $("#toggle-sidebar").focus();
}

function setInspector(visible, focus = true) {
  if (visible && compact.matches) workspace.dataset.sidebar = "false";
  workspace.dataset.inspector = String(visible);
  $("#inspector").hidden = !visible;
  $("#toggle-inspector").setAttribute("aria-expanded", String(visible));
  syncNavigator();
  if (focus) (visible ? $("#close-inspector") : $("#toggle-inspector")).focus();
}

function setView(name, focus = false) {
  $$("[data-view]").forEach((tab) => {
    const active = tab.dataset.view === name;
    tab.setAttribute("aria-selected", String(active));
    tab.tabIndex = active ? 0 : -1;
    $(`#view-${tab.dataset.view}`).hidden = !active;
  });
  if (focus) $(`#tab-${name}`).focus();
  announce(`${name[0].toUpperCase()}${name.slice(1)} view`);
}

function setDetail(name) {
  $$("[data-detail]").forEach((tab) => {
    const active = tab.dataset.detail === name;
    tab.setAttribute("aria-selected", String(active));
    tab.tabIndex = active ? 0 : -1;
    $(`#inspector-${tab.dataset.detail}`).hidden = !active;
  });
}

function renderSessions() {
  const nav = $("#conversations");
  nav.replaceChildren();
  currentProject().sessions.forEach((session, index) => {
    if (index === 2) {
      const label = document.createElement("div");
      label.className = "section-label recent-label";
      label.textContent = "TODAY";
      nav.append(label);
    }
    const button = document.createElement("button");
    button.className = "conversation-link";
    if (index === currentProject().selected)
      button.setAttribute("aria-current", "page");
    const text = document.createElement("span");
    const title = document.createElement("strong");
    title.textContent = session.title;
    const detail = document.createElement("small");
    detail.textContent = session.detail;
    text.append(title, detail);
    button.append(icon("chat"), text);
    button.addEventListener("click", () => selectSession(index));
    nav.append(button);
  });
}

function appendUser(text) {
  const message = document.createElement("div");
  message.className = "user-message";
  message.textContent = text;
  $("#transcript").append(message);
}

function appendAgent(text) {
  const heading = document.createElement("div");
  heading.className = "agent-heading";
  const mark = document.createElement("span");
  mark.className = "agent-mark";
  mark.append(icon("spark"));
  const title = document.createElement("strong");
  title.textContent = "OpenCode";
  const sample = document.createElement("span");
  sample.textContent = "Sample response";
  heading.append(mark, title, sample);
  const body = document.createElement("div");
  body.className = "agent-message";
  body.textContent = text;
  $("#transcript").append(heading, body);
}

function renderTranscript() {
  const session = currentSession();
  $("#conversation-title").textContent = session.title;
  $("#conversation-subtitle").textContent = session.empty
    ? "A fresh page. Your next idea."
    : "A simpler path from an idea to something great.";
  $("#transcript").replaceChildren();
  if (!session.empty) {
    appendUser(
      "Let's make this workspace feel lighter. Fewer places to look, more room for the work.",
    );
    appendAgent(
      "One project navigator. Three clear destinations. The details stay nearby, without competing for your attention. Glass belongs around the work — not between you and what you're reading.",
    );
    // Static, trusted fixture markup only. User input is always rendered as text.
    const card = document.createElement("div");
    card.className = "plan-card";
    card.innerHTML =
      '<div class="plan-card-header"><strong>A quieter workspace</strong><span>Sample plan · 1 of 2</span></div><div class="plan-card-content"><span class="step-check">✓</span><div><strong>Give every action a clear home</strong><small>Work, review and activity. Nothing extra.</small></div><button class="text-button" type="button">Review changes ↗</button></div>';
    $("button", card).addEventListener("click", () => setView("changes", true));
    $("#transcript").append(card);
  }
  session.messages.forEach((message) => {
    appendUser(message);
    appendAgent(
      "Demo reply — your message stays in this page only. In the native application this space would show the connected agent's actual response, tool calls and explicit approvals.",
    );
  });
  $("#prompt").value = session.draft;
  updateSend();
}

function selectSession(index) {
  currentProject().selected = index;
  renderSessions();
  renderTranscript();
  setView("work");
  if (narrow.matches) setSidebar(false);
  $("#prompt").focus();
}

function selectProject(id) {
  projectId = id;
  $("#project-name").textContent = currentProject().name;
  $("#scope-project").textContent = currentProject().name;
  $(".project-picker .project-icon").textContent = id[0];
  renderSessions();
  renderTranscript();
  setView("work");
  if (narrow.matches) setSidebar(false);
  announce(`${currentProject().name} demo project selected. This Mac, local.`);
}

function newConversation() {
  const project = currentProject();
  project.sessions.push({
    title: "A new idea",
    detail: "Draft · This Mac",
    draft: "",
    messages: [],
    empty: true,
  });
  selectSession(project.sessions.length - 1);
}

function updateSend() {
  $(".send-button").disabled = !$("#prompt").value.trim();
}

function renderFiles() {
  const nav = $("#changed-files");
  nav.replaceChildren();
  files.forEach((file, index) => {
    const button = document.createElement("button");
    button.append(icon("file"));
    const title = document.createElement("span");
    title.textContent = file.name;
    button.append(title);
    button.setAttribute("aria-current", String(index === selectedFile));
    if (reviewed.has(index)) {
      const check = document.createElement("span");
      check.className = "file-reviewed";
      check.textContent = "✓";
      check.setAttribute("aria-label", "Reviewed");
      button.append(check);
    }
    button.addEventListener("click", () => {
      selectedFile = index;
      renderFiles();
    });
    nav.append(button);
  });
  $("#diff-filename").textContent = `src/${files[selectedFile].name}`;
  $("#diff-content").replaceChildren();
  files[selectedFile].lines.forEach((line) => {
    const row = document.createElement("span");
    row.className = `diff-line${line.startsWith("+") ? " added" : line.startsWith("-") ? " removed" : ""}`;
    row.textContent = line || " ";
    $("#diff-content").append(row);
  });
  $("#review-progress").textContent = `${reviewed.size} of 3 files reviewed`;
  $("#mark-reviewed").textContent = reviewed.has(selectedFile)
    ? "Mark as unreviewed"
    : "Mark as reviewed";
}

function commandItems() {
  return [
    ...["work", "changes", "activity"].map((view) => ({
      label: `Open ${view[0].toUpperCase()}${view.slice(1)}`,
      group: "Destination",
      run: () => setView(view, true),
    })),
    { label: "New conversation", group: "Command", run: newConversation },
    {
      label: "Toggle context inspector",
      group: "Command",
      run: () => setInspector(workspace.dataset.inspector !== "true"),
    },
    {
      label: "Settings",
      group: "Command",
      run: () => openDialog("#settings-dialog", $("#open-commands")),
    },
    ...Object.entries(projects).map(([id, project]) => ({
      label: project.name,
      group: "Project · Local",
      run: () => selectProject(id),
    })),
    ...currentProject().sessions.map((session, index) => ({
      label: session.title,
      group: "Conversation",
      run: () => selectSession(index),
    })),
    ...files.map((file, index) => ({
      label: `src/${file.name}`,
      group: "Sample file",
      run: () => openFile(index),
    })),
  ];
}

function renderCommands() {
  const query = $("#command-query").value.trim().toLowerCase();
  const results = $("#command-results");
  results.replaceChildren();
  const matches = commandItems().filter((item) =>
    `${item.label} ${item.group}`.toLowerCase().includes(query),
  );
  matches.forEach((item) => {
    const button = document.createElement("button");
    const label = document.createElement("span");
    label.textContent = item.label;
    const group = document.createElement("small");
    group.textContent = item.group;
    button.append(label, group);
    button.addEventListener("click", () => {
      $("#command-dialog").close();
      item.run();
    });
    results.append(button);
  });
  if (!matches.length) {
    const empty = document.createElement("p");
    empty.textContent = "No matches. Try a destination, project or file name.";
    results.append(empty);
  }
}

function openCommands(opener = document.activeElement) {
  $("#command-query").value = "";
  renderCommands();
  openDialog("#command-dialog", opener);
  $("#command-query").focus();
}

function openFile(index) {
  selectedFile = index;
  renderFiles();
  setView("changes", true);
  if (narrow.matches) setInspector(false, false);
}

$("#theme").addEventListener("change", setTheme);
dark.addEventListener("change", setTheme);
$("#glass").addEventListener("change", (event) => {
  root.dataset.glass = String(event.target.checked);
});
$("#reduce-motion").addEventListener("change", (event) => {
  root.dataset.motion = String(!event.target.checked);
});
$("#toggle-sidebar").addEventListener("click", () =>
  setSidebar($("#toggle-sidebar").getAttribute("aria-expanded") !== "true"),
);
$("#toggle-inspector").addEventListener("click", () =>
  setInspector(workspace.dataset.inspector !== "true"),
);
$("#close-inspector").addEventListener("click", () => setInspector(false));
$("#choose-project").addEventListener("click", (event) =>
  openDialog("#project-dialog", event.currentTarget),
);
$("#new-conversation").addEventListener("click", newConversation);
$("#open-commands").addEventListener("click", (event) =>
  openCommands(event.currentTarget),
);
$("#open-settings").addEventListener("click", (event) =>
  openDialog("#settings-dialog", event.currentTarget),
);
$("#attach-context").addEventListener("click", () => {
  setDetail("files");
  setInspector(true);
});
$("#draft-commit").addEventListener("click", (event) =>
  openDialog("#commit-dialog", event.currentTarget),
);
$("#command-query").addEventListener("input", renderCommands);
$("#command-query").addEventListener("keydown", (event) => {
  if (event.key === "ArrowDown") {
    event.preventDefault();
    $("button", $("#command-results"))?.focus();
  }
});
$("#mark-reviewed").addEventListener("click", () => {
  reviewed.has(selectedFile)
    ? reviewed.delete(selectedFile)
    : reviewed.add(selectedFile);
  renderFiles();
});
$("#prompt").addEventListener("input", () => {
  currentSession().draft = $("#prompt").value;
  updateSend();
});
$("#composer").addEventListener("submit", (event) => {
  event.preventDefault();
  const text = $("#prompt").value.trim();
  if (!text) return;
  currentSession().messages.push(text);
  currentSession().draft = "";
  renderTranscript();
  $("#conversation-scroll").scrollTop = $("#conversation-scroll").scrollHeight;
  announce("Demo reply added. No agent was called.");
});
$$("[data-close-dialog]").forEach((button) =>
  button.addEventListener("click", () => button.closest("dialog").close()),
);
$$("[data-project]").forEach((button) =>
  button.addEventListener("click", () => {
    $("#project-dialog").close();
    selectProject(button.dataset.project);
  }),
);
$$("[data-view]").forEach((button) =>
  button.addEventListener("click", () => setView(button.dataset.view)),
);
$$("[data-go-view]").forEach((button) =>
  button.addEventListener("click", () => {
    setView(button.dataset.goView, true);
    if (narrow.matches) setInspector(false, false);
  }),
);
$$("[data-detail]").forEach((button) =>
  button.addEventListener("click", () => setDetail(button.dataset.detail)),
);
$$("[data-open-file]").forEach((button) =>
  button.addEventListener("click", () =>
    openFile(Number(button.dataset.openFile)),
  ),
);
$$('[role="tablist"]').forEach((list) =>
  list.addEventListener("keydown", (event) => {
    const tabs = $$('[role="tab"]', list);
    const index = tabs.indexOf(document.activeElement);
    if (index < 0) return;
    let next;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft")
      next = (index + tabs.length - 1) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    event.preventDefault();
    tabs[next].click();
    tabs[next].focus();
  }),
);
document.addEventListener("keydown", (event) => {
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
});
function adaptLayout() {
  if (narrow.matches) setSidebar(false);
  syncNavigator();
}
narrow.addEventListener("change", adaptLayout);
compact.addEventListener("change", syncNavigator);
renderSessions();
renderTranscript();
renderFiles();
setTheme();
adaptLayout();
// Decorative icons must never compete with their controls' accessible names.
$$("svg:not(.icon-definitions)").forEach((svg) =>
  svg.setAttribute("aria-hidden", "true"),
);
