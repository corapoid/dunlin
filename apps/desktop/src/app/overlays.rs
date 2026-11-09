use super::*;
use dunlin_ui::component::ThemeStyled as _;
use dunlin_ui::component::menu::DropdownMenu as _;
use dunlin_ui::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
};

impl DunlinApp {
    pub(super) fn rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut rail = div()
            .id("navigation-rail")
            .relative()
            .w(px(52.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .py(px(9.))
            .gap(px(3.));
        #[cfg(debug_assertions)]
        {
            rail = rail.children(crate::workspace_ui_smoke::probe("navigation-rail"));
        }

        let mut items = vec![
            (
                SidebarSection::Dashboard,
                IconName::ChartNoAxesColumn,
                0,
                false,
            ),
            (
                SidebarSection::Projects,
                IconName::LayoutGrid,
                self.projects.len(),
                false,
            ),
            (SidebarSection::Files, IconName::Files, 0, false),
            (SidebarSection::Search, IconName::Search, 0, false),
            (SidebarSection::SourceControl, IconName::GitBranch, 4, true),
        ];
        items.push((
            SidebarSection::Sessions,
            IconName::Clock3,
            self.sessions.len(),
            false,
        ));
        items.push((
            SidebarSection::Knowledge,
            IconName::BookOpen,
            self.knowledge.graph.objects.len(),
            false,
        ));

        for (section, icon_name, count, has_badge) in items {
            let active = self.section == section && !self.settings_open;
            let mut button =
                navigation_button(("rail", section as usize), icon_name, section.label())
                    .relative()
                    .size(px(36.))
                    .selected(active)
                    .toggled(active)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.section = section;
                        if section != SidebarSection::Dashboard {
                            this.sidebar_collapsed = false;
                        }
                        this.settings_open = false;
                        if section == SidebarSection::Files {
                            this.focus_explorer(window, cx);
                        }
                        if section == SidebarSection::SourceControl {
                            this.open_git_history(window, cx);
                            if !this.git_loading {
                                this.refresh_git_status(cx);
                            }
                        }
                        this.persist_settings(cx);
                        cx.notify();
                    }));
            #[cfg(debug_assertions)]
            if section == SidebarSection::Projects {
                button = button.children(crate::workspace_ui_smoke::probe("rail-projects"));
            }
            if has_badge && count > 0 {
                button = button.child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(p.amber)
                        .border_1()
                        .border_color(p.panel),
                );
            }
            rail = rail.child(button);
        }

        rail.child(div().flex_1())
            .child(
                navigation_button("rail-settings", IconName::Settings, "Settings · ⌘,")
                    .relative()
                    .size(px(36.))
                    .selected(self.settings_open)
                    .toggled(self.settings_open)
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe("rail-settings")
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::native_menu::Settings), cx)
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn takeover_current_session(&mut self, cx: &mut Context<Self>) {
        let session = self.session().clone();
        let Some(host_id) = session.origin.host_id().map(str::to_string) else {
            self.set_notice("Only remote sessions can be taken over", cx);
            return;
        };
        let Some(host) = self.remote_host(&host_id) else {
            self.set_notice("Remote server is not configured", cx);
            return;
        };
        let token = match self.remote_token(&host_id) {
            Ok(token) => token,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let session_id = session
            .acp_session_id
            .clone()
            .unwrap_or_else(|| session.id.clone());
        let session_for_load = session_id.clone();
        let expected_project = self.project().id.clone();
        self.notice = Some("Taking over the remote session…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { remote::takeover_session(&host.url, &token, &session_id) })
                .await;
            this.update(cx, |this, cx| {
                if this.project().id != expected_project {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.harness.pending_remote_session = Some(PendingRemoteSession::Load {
                            session_id: session_for_load,
                        });
                        this.disconnect_harness(cx);
                        this.connect_harness(cx);
                        this.notice = Some("Took over the remote session".into());
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Right-click takeover keeps the removed session menu's remote recovery
    /// path without restoring the title-bar menu.
    pub(crate) fn takeover_session_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(index) else {
            return;
        };
        if !session.origin.is_remote() {
            self.set_notice("Only remote sessions can be taken over", cx);
            return;
        }
        if index != self.selected_session {
            self.interrupt_live_turn(cx);
            self.stash_transcript();
            self.selected_session = index;
            let id = self.session().id.clone();
            self.load_transcript(&id);
        }
        self.takeover_current_session(cx);
    }

    fn onboarding(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        // Two steps: pick a default harness, then create the first project.
        // Older saved progress is clamped so nobody sees a redundant final page.
        let step = self.onboarding_step.min(1);

        if step == 0 {
            let ready: Vec<&'static str> = self
                .harness_discovery
                .iter()
                .filter(|detected| detected.adapter.is_some())
                .map(|detected| detected.name)
                .collect();
            if !ready.is_empty() && !ready.contains(&self.default_harness.as_str()) {
                self.default_harness = ready[0].to_string();
                self.persist_settings(cx);
            }
        }

        let title = if step == 0 {
            "Choose a harness"
        } else {
            "Create your first project"
        };
        let subtitle = if step == 0 {
            "Choose an agent for new conversations. Installed CLIs may need a separate ACP adapter."
        } else {
            "Pick a folder and start working. Nothing is uploaded; your project stays on this Mac."
        };

        let mut dots = div().flex().items_center().gap(px(5.));
        for index in 0..2 {
            dots = dots.child(div().size(px(5.)).rounded_full().bg(if index <= step {
                p.fg
            } else {
                p.border_strong
            }));
        }

        let mut body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .items_center()
            .justify_center()
            .gap(px(18.))
            .child(
                div()
                    .size(px(38.))
                    .rounded(px(dunlin_ui::radii::DRAG_PREVIEW))
                    .bg(p.fg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::Bird).size(px(19.)).text_color(p.bg)),
            )
            .child(dots)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_size(px(19.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(title),
                    )
                    .child(
                        div()
                            .max_w(px(480.))
                            .text_center()
                            .text_size(px(12.0))
                            .line_height(relative(1.55))
                            .text_color(p.muted_fg)
                            .child(subtitle),
                    ),
            );

        if step == 0 {
            let available = self
                .harness_discovery
                .iter()
                .filter(|detected| {
                    detected.visible_in_onboarding(self.onboarding_show_all_harnesses)
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut rows = div()
                .id("onboarding-harness-list")
                .w(px(460.))
                .max_w_full()
                .max_h(px(260.))
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(6.));
            for detected in available {
                let name = detected.name;
                let installed = detected.adapter.is_some();
                let selected = installed && self.default_harness == name;
                let name_owned = name.to_string();
                let name_for_button = name_owned.clone();
                let row = action_row(
                    format!("onboarding-harness:{name}"),
                    format!("Use {name} harness"),
                )
                .flex_1()
                .min_w(px(0.))
                .w_auto()
                .selected(selected)
                .toggled(selected)
                .disabled(!installed)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.default_harness = name_owned.clone();
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(16.))
                        .flex_shrink_0()
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { p.fg } else { p.border_strong })
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(selected.then(|| div().size(px(7.)).rounded_full().bg(p.fg))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(dot(
                                    if installed {
                                        p.green
                                    } else if detected.cli.is_some() {
                                        p.amber
                                    } else {
                                        p.muted_fg
                                    },
                                    4.0,
                                ))
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.muted_fg)
                                        .child(detected.status()),
                                ),
                        ),
                );
                let row = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .flex_shrink_0()
                    .child(row)
                    .child(div().w(px(96.)).flex().justify_end().child(if selected {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(Icon::new(IconName::Check).size(px(12.)).text_color(p.fg))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .child("Default"),
                            )
                            .into_any_element()
                    } else if installed {
                        div()
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .child("Set default")
                            .into_any_element()
                    } else {
                        onboarding_secondary_button(
                            format!("onboarding-harness-install:{name}"),
                            "Set up…",
                            &p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.show_harness_setup(&name_for_button, cx);
                        }))
                        .into_any_element()
                    }));
                rows = rows.child(row);
            }
            body = body.child(rows);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        onboarding_text_button(
                            "onboarding-harness-show-all",
                            if self.onboarding_show_all_harnesses {
                                "Show detected only"
                            } else {
                                "Choose another / Show all"
                            },
                            &p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.onboarding_show_all_harnesses =
                                !this.onboarding_show_all_harnesses;
                            cx.notify();
                        })),
                    )
                    .child(
                        onboarding_text_button(
                            "onboarding-harness-rescan",
                            if self.harness_scanning {
                                "Scanning…"
                            } else {
                                "Scan again"
                            },
                            &p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.rescan_harnesses(cx))),
                    ),
            );
            if let Some(notice) = &self.harness_setup_notice {
                body = body.child(
                    div()
                        .max_w(px(440.))
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(notice.clone()),
                );
            }
            if self
                .harness_discovery
                .iter()
                .all(|detected| detected.adapter.is_none())
            {
                body = body.child(
                    div()
                        .max_w(px(440.))
                        .text_center()
                        .text_size(px(11.0))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("No ACP adapter was detected. Use Set up to view installation instructions, then Scan again. You can also do this later in Settings › Agents."),
                );
            }
        }

        if step == 0 {
            body = body.child(
                div()
                    .w(px(460.))
                    .flex()
                    .items_center()
                    .child(div().flex_1())
                    .child(
                        onboarding_primary_button("onboarding-continue", "Continue", &p).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.onboarding_step = 1;
                                this.persist_settings(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            );
        } else {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        onboarding_primary_button(
                            "onboarding-create-project",
                            "Create first project…",
                            &p,
                        )
                        .h(px(38.))
                        .px(px(18.))
                        .text_size(px(12.5))
                        .dropdown_menu_with_anchor(Anchor::TopLeft, {
                            let app = cx.entity().downgrade();
                            move |menu, _, _| crate::project_import::source_menu(menu, app.clone())
                        }),
                    )
                    .child(
                        onboarding_text_button("onboarding-back", "Back", &p).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.onboarding_step = 0;
                                this.persist_settings(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }

        body.into_any_element()
    }

    pub(crate) fn finish_terminal_resize(&mut self, cx: &mut Context<Self>) {
        if self.terminal_resize.take().is_some() {
            self.persist_settings(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_terminal_dock(
        &mut self,
        dock: crate::app::TerminalDock,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_dock != dock {
            self.terminal_dock = dock;
            self.persist_settings(cx);
            cx.notify();
        }
    }

    pub(crate) fn toggle_bottom_panel(
        &mut self,
        panel: crate::app::BottomPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::app::BottomPanel;
        if self.bottom_panel == panel {
            self.bottom_panel = BottomPanel::Hidden;
            self.persist_settings(cx);
            cx.notify();
            return;
        }
        self.bottom_panel = panel;
        match panel {
            BottomPanel::Terminal => {
                if self.terminal.tabs.iter().all(|tab| tab.workspace)
                    && (self.project().is_remote() || self.project().trusted)
                    && let Err(error) = self.open_terminal_tab(cx)
                {
                    self.terminal.error = Some(error);
                }
                self.terminal.focus.focus(window, cx);
            }
            BottomPanel::Run => self.ensure_run_detection(cx),
            BottomPanel::Secrets => self.refresh_vault(cx),
            BottomPanel::Agents => self.poll_coordination(cx),
            BottomPanel::Hidden => {}
        }
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn close_bottom_panel(&mut self, cx: &mut Context<Self>) {
        self.bottom_panel = crate::app::BottomPanel::Hidden;
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn bottom_panel_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::app::BottomPanel;
        let p = palette(cx);
        let viewport_width = f32::from(window.viewport_size().width);
        let viewport_height = f32::from(window.viewport_size().height);
        let max_height = (viewport_height * 0.62).max(160.);
        let height = self.bottom_panel_height.clamp(160., max_height);
        let width = self
            .bottom_panel_width
            .clamp(260., (viewport_width * 0.55).max(260.));
        let mut tabs = Vec::new();
        let mut selected = 0;
        for (panel, label) in [
            (BottomPanel::Terminal, "Terminal"),
            (BottomPanel::Run, "Run"),
            (BottomPanel::Secrets, "Secrets"),
            (BottomPanel::Agents, "Agents"),
        ] {
            let active = self.bottom_panel == panel;
            if active {
                selected = tabs.len();
            }
            tabs.push(
                document_tab(label)
                    .background_motion(!self.reduce_motion)
                    .focusable()
                    .tab_stop(true)
                    .aria_label(format!("Open {label} panel"))
                    .prefix(
                        Icon::new(match panel {
                            BottomPanel::Terminal | BottomPanel::Hidden => IconName::Terminal,
                            BottomPanel::Run => IconName::SquareTerminal,
                            BottomPanel::Secrets => IconName::KeyRound,
                            BottomPanel::Agents => IconName::Bot,
                        })
                        .size(px(14.)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.bottom_panel != panel {
                            this.toggle_bottom_panel(panel, window, cx);
                        }
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            if this.bottom_panel != panel {
                                this.toggle_bottom_panel(panel, window, cx);
                            }
                        }
                    })),
            );
        }
        let menu = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .child(
                div().flex().items_center().gap(px(2.)).children(
                    [
                        (crate::app::TerminalDock::Bottom, "▁", "Bottom"),
                        (crate::app::TerminalDock::Left, "◧", "Left"),
                        (crate::app::TerminalDock::Right, "◨", "Right"),
                    ]
                    .map(|(dock, glyph, label)| {
                        let active = self.terminal_dock == dock;
                        segment_button(format!("terminal-dock-position:{label}"), glyph, active)
                            .accessibility_label(format!("Place terminal panel at {label}"))
                            .tooltip(format!("Move panel to the {label} edge"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_terminal_dock(dock, cx)),
                            )
                            .into_any_element()
                    }),
                ),
            )
            .child(
                icon_button("bottom-panel-close", IconName::X, "Close bottom panel")
                    .on_click(cx.listener(|this, _, _, cx| this.close_bottom_panel(cx))),
            );
        let header = div()
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .overflow_hidden()
            .rounded_t(px(dunlin_ui::scale::WORKSPACE_RADIUS))
            .child(
                document_tabs("bottom-panel-tabs")
                    .children(tabs)
                    .selected_index(selected)
                    .ml(-px(dunlin_ui::radii::FOLDER_TAB))
                    .suffix(menu)
                    .flex_1()
                    .min_w(px(0.)),
            );
        let body = match self.bottom_panel {
            BottomPanel::Terminal => crate::terminal::render(self, window, cx),
            BottomPanel::Run => crate::run::render(self, window, cx),
            BottomPanel::Secrets => crate::vault::panel::render(self, window, cx),
            BottomPanel::Agents => crate::coordination::render(self, window, cx),
            BottomPanel::Hidden => div().into_any_element(),
        };
        let dock = self.terminal_dock;
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .bg(p.card)
                    .rounded_tr(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                    .when(selected > 0, |surface| {
                        surface.rounded_tl(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                    })
                    .rounded_b(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                    .pt(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                    .pb(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                    .overflow_hidden()
                    .child(body),
            );
        let horizontal_handle = div()
            .id("terminal-dock-resize")
            .aria_label("Drag to resize the terminal panel")
            .h(px(4.))
            .w_full()
            .flex_shrink_0()
            .occlude()
            .cursor(CursorStyle::ResizeUpDown)
            .hover(move |style| style.bg(p.border_strong))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.bottom_panel_height = crate::app::default_bottom_panel_height();
                        this.persist_settings(cx);
                    } else {
                        this.terminal_resize =
                            Some((f32::from(event.position.y), this.bottom_panel_height));
                    }
                    cx.notify();
                }),
            );
        let vertical_handle = div()
            .id("terminal-dock-resize")
            .aria_label("Drag to resize the terminal panel")
            .w(px(9.))
            .h_full()
            .flex_shrink_0()
            .occlude()
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(move |style| style.bg(p.border_strong))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.bottom_panel_width = crate::app::default_bottom_panel_width();
                        this.persist_settings(cx);
                    } else {
                        this.terminal_resize =
                            Some((f32::from(event.position.x), this.bottom_panel_width));
                    }
                    cx.notify();
                }),
            );
        match dock {
            crate::app::TerminalDock::Bottom => div()
                .h(px(height))
                .w_full()
                .flex_shrink_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .rounded_b(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                .child(horizontal_handle)
                .child(content)
                .into_any_element(),
            crate::app::TerminalDock::Left => div()
                .h_full()
                .w(px(width))
                .flex_shrink_0()
                .overflow_hidden()
                .flex()
                .child(content)
                .child(vertical_handle)
                .into_any_element(),
            crate::app::TerminalDock::Right => div()
                .h_full()
                .w(px(width))
                .flex_shrink_0()
                .overflow_hidden()
                .flex()
                .child(vertical_handle)
                .child(content)
                .into_any_element(),
        }
    }

    pub(super) fn main_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.has_modal_overlay() || self.section == SidebarSection::Dashboard {
            self.workbench.new_tab_menu = None;
            self.workbench.new_tab_press = None;
        }
        if self.section == SidebarSection::Dashboard {
            return crate::dashboard::render(self, cx);
        }
        if self.section == SidebarSection::Knowledge {
            return crate::knowledge::render(self, window, cx);
        }
        if self.projects.is_empty() {
            return self.onboarding(cx);
        }
        self.sync_workspace_panes(window, cx);
        let count = if self.workbench.panes.enabled { 2 } else { 1 };
        let mut row = div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .gap(px(6.));
        for pane in 0..count {
            let (tabs, rounded_left) = self.render_workbench_tabs(pane, cx);
            let content =
                if self.workbench.panes.enabled && self.workbench.panes.active[pane].is_none() {
                    crate::workbench::empty_workspace(cx)
                } else if pane == self.workbench.panes.focused {
                    self.workspace_content(window, cx)
                } else {
                    self.render_inactive_pane(pane, window, cx)
                };
            let edge_to_edge = matches!(
                self.workbench.panes.active[pane],
                Some(
                    crate::workspace_panes::TabKey::File(_)
                        | crate::workspace_panes::TabKey::Terminal(_)
                        | crate::workspace_panes::TabKey::Git(_)
                )
            );
            let surface = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        (pane == 0)
                            .then(|| crate::workspace_ui_smoke::probe("reading-surface"))
                            .flatten()
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .bg(palette(cx).card)
                .when(!edge_to_edge, |surface| {
                    surface.py(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                })
                .rounded_tr(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                .when(rounded_left, |surface| {
                    surface.rounded_tl(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                })
                .rounded_b(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                .overflow_hidden()
                .child(content);
            row = row.child(
                div()
                    .id(format!("workspace-pane:{pane}"))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .flex_basis(px(0.))
                    .relative()
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe(if pane == 0 {
                                "workspace-pane-0"
                            } else {
                                "workspace-pane-1"
                            })
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_hidden()
                    .capture_any_mouse_down(cx.listener(move |this, _, window, cx| {
                        if !this.focus_workspace_pane(pane, window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .capture_key_down(cx.listener(move |this, _, window, cx| {
                        if !this.focus_workspace_pane(pane, window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .child(tabs)
                    .child(surface),
            );
        }
        row.into_any_element()
    }

    pub(crate) fn workspace_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.spec_tools_enabled && self.spec.is_active_for(self.project()) {
            crate::spec::render(self, cx)
        } else if self.git_history.is_active_for(self.project()) {
            crate::git_history::render(self, cx)
        } else if self.workbench.show_file
            && let Some(inspector) = &self.inspector
        {
            self.inspector_panel(inspector, cx)
        } else if let Some(id) = self.workbench.terminal.clone().filter(|id| {
            self.terminal
                .tabs
                .iter()
                .any(|tab| tab.id == *id && tab.project == self.project().id)
        }) {
            crate::terminal::render_tab(self, &id, window, cx)
        } else if self.workbench.closed_chats.contains(&self.session().id) {
            crate::workbench::empty_workspace(cx)
        } else {
            match self.view {
                MainView::Chat => views::chat::render(self, window, cx),
                MainView::Graph => views::graph::render(self, window, cx),
                MainView::Trace => views::trace::render(self, window, cx),
            }
        }
    }

    fn editor_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(pending) = self.pending_editor.as_ref() else {
            return div().into_any_element();
        };
        let name = self
            .inspector
            .as_ref()
            .map(|inspector| inspector.name.clone())
            .unwrap_or_else(|| "this file".into());
        let next = match pending {
            PendingEditor::Close => "closing the editor".to_string(),
            PendingEditor::SwitchProject { .. } => "switching projects".to_string(),
            PendingEditor::RemoveProject { .. } => "removing this project".to_string(),
        };

        div()
            .id("editor-prompt-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_pending_editor(cx)))
            .child(
                dialog_surface(p)
                    .id("editor-prompt")
                    .child(crate::effects::panel_probe(
                        &self.overlay_backdrop,
                        SurfaceGeometry::DIALOG,
                        false,
                    ))
                    .w(px(400.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .flex_shrink_0()
                                    .rounded(px(dunlin_ui::radii::PANEL))
                                    .bg(p.amber_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::FileCode, 14.0, p.amber)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child("Unsaved changes"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .line_height(relative(1.5))
                                            .text_color(p.muted_fg)
                                            .child(format!(
                                                "Save changes to “{name}” before {next}?"
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(7.))
                            .child(crate::gui_probe::target(
                                "editor-cancel",
                                dunlin_ui::button("editor-cancel").label("Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_pending_editor(cx);
                                    }),
                                ),
                            ))
                            .child(crate::gui_probe::target(
                                "editor-discard",
                                dunlin_ui::button("editor-discard")
                                    .danger()
                                    .outline()
                                    .label("Discard")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_pending_editor(window, cx);
                                    })),
                            ))
                            .child(crate::gui_probe::target(
                                "editor-save",
                                dunlin_ui::button("editor-save")
                                    .primary()
                                    .label("Save")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_pending_editor(window, cx);
                                    })),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn diff_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(view) = self.diff_view.as_ref() else {
            return div().into_any_element();
        };
        let path = view.path.clone();
        let open_path = path.clone();
        let name = view.name.clone();
        let staged = view.staged;
        let error = view.error.clone();

        let body = match error {
            Some(error) => div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(7.))
                .child(icon(IconName::FileDiff, 24.0, p.muted_fg))
                .child(div().text_size(px(12.5)).text_color(p.fg).child(error))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted_fg)
                        .child("This file cannot be previewed as a patch."),
                )
                .into_any_element(),
            None => div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(Diff::new(&view.state).line_number(true).soft_wrap(false))
                .into_any_element(),
        };

        div()
            .id("diff-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx)))
            .child(
                SurfaceGeometry::DIALOG
                    .paint(p.card, p.border_strong)
                    .id("diff-dialog")
                    .child(crate::effects::panel_probe(
                        &self.overlay_backdrop,
                        SurfaceGeometry::DIALOG,
                        false,
                    ))
                    .w(px(880.))
                    .h(px(560.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(52.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(p.border)
                            .child(
                                div()
                                    .size(px(28.))
                                    .flex_shrink_0()
                                    .rounded(px(dunlin_ui::radii::CONTROL))
                                    .bg(if staged { p.green_soft } else { p.amber_soft })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(
                                        IconName::FileDiff,
                                        14.0,
                                        if staged { p.green } else { p.amber },
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .truncate()
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .font_family(dunlin_ui::theme::code_font())
                                            .text_color(p.muted_fg)
                                            .truncate()
                                            .child(path.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(19.))
                                    .px(px(6.))
                                    .rounded(px(dunlin_ui::radii::CODE))
                                    .bg(if staged { p.green_soft } else { p.amber_soft })
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_family(dunlin_ui::theme::code_font())
                                            .text_color(if staged { p.green } else { p.amber })
                                            .child(if staged { "STAGED" } else { "WORKTREE" }),
                                    ),
                            )
                            .child(
                                dunlin_ui::button("diff-open-file")
                                    .icon(IconName::ExternalLink)
                                    .label("Open file")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_diff(window, cx);
                                        this.open_file(&open_path, window, cx);
                                    })),
                            )
                            .child(
                                icon_button("diff-close", IconName::X, "Close diff").on_click(
                                    cx.listener(|this, _, window, cx| this.close_diff(window, cx)),
                                ),
                            ),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn remote_picker_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(picker) = self.remote_picker.as_ref() else {
            return div().into_any_element();
        };
        let host_id = picker.host_id.clone();
        let host_name = picker.host_name.clone();
        let loading = picker.loading;
        let error = picker.error.clone();
        let projects = picker.projects.clone();

        let mut list = div().flex().flex_col().gap(px(2.));
        if loading {
            list = list.child(
                div()
                    .py(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .child(icon(IconName::LoaderCircle, 13.0, p.muted_fg))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.muted_fg)
                            .child("Loading projects…"),
                    ),
            );
        } else if let Some(error) = error {
            list = list.child(
                div()
                    .py(px(18.))
                    .px(px(10.))
                    .rounded(px(dunlin_ui::radii::ROW))
                    .bg(p.red_soft)
                    .text_size(px(11.5))
                    .text_color(p.red)
                    .child(error),
            );
        } else if projects.is_empty() {
            list = list.child(
                div()
                    .py(px(24.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(p.fg)
                            .child("No projects on this relay"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.muted_fg)
                            .child("Add one with `dunlin-relay project add`."),
                    ),
            );
        } else {
            for info in &projects {
                let info = info.clone();
                let info_for_click = info.clone();
                let host = host_id.clone();
                list = list.child(
                    action_row(
                        format!("remote-project:{}", info.id),
                        format!("Add remote project {}", info.name),
                    )
                    .relative()
                    .children(crate::gui_probe::marker_named(format!(
                        "remote-project:{}",
                        info.id
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_remote_project(&host, &info_for_click, cx);
                    }))
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(dunlin_ui::radii::ROW))
                            .bg(p.blue_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Folder, 12.0, p.blue)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .child(info.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(info.path.clone()),
                            ),
                    )
                    .child(icon(IconName::ChevronRight, 11.0, p.muted_fg)),
                );
            }
        }

        div()
            .id("remote-picker-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.remote_picker = None;
                cx.notify();
            }))
            .child(
                dialog_surface(p)
                    .id("remote-picker")
                    .popover_style(cx)
                    .occlude()
                    .w(px(440.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(28.))
                                    .rounded(px(dunlin_ui::radii::CONTROL))
                                    .bg(p.blue_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Cloud, 13.0, p.blue)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child("Add remote project"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(p.muted_fg)
                                            .child(host_name),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                icon_button(
                                    "remote-picker-close",
                                    IconName::X,
                                    "Close remote project picker",
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.remote_picker = None;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .id("remote-picker-list")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    fn new_file_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let renaming = self
            .explorer
            .creation
            .as_ref()
            .is_some_and(|creation| creation.rename.is_some());
        let directory = self
            .explorer
            .creation
            .as_ref()
            .is_some_and(|creation| creation.directory);
        let location = self
            .explorer
            .creation
            .as_ref()
            .map(|creation| display_path(&creation.root, Path::new(&creation.parent)))
            .unwrap_or_default();
        div()
            .id("new-file-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                if this.explorer.busy {
                    return;
                }
                this.new_file_prompt = false;
                cx.notify();
            }))
            .child(
                dialog_surface(p)
                    .id("new-file-modal")
                    .child(crate::effects::panel_probe(
                        &self.overlay_backdrop,
                        SurfaceGeometry::DIALOG,
                        false,
                    ))
                    .w(px(380.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(if renaming {
                                "Rename item"
                            } else if directory {
                                "New folder"
                            } else {
                                "New file"
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "Location: {}. Existing items are never replaced.",
                                if location.is_empty() { "." } else { &location }
                            )),
                    )
                    .child(dunlin_ui::input(&self.new_file_input).aria_label("File or folder name"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(7.))
                            .child(
                                dunlin_ui::button("new-file-cancel")
                                    .label("Cancel")
                                    .disabled(self.explorer.busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.explorer.busy {
                                            return;
                                        }
                                        this.new_file_prompt = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                dunlin_ui::button("new-file-create")
                                    .primary()
                                    .disabled(self.explorer.busy)
                                    .label(if self.explorer.busy {
                                        if renaming {
                                            "Renaming…"
                                        } else {
                                            "Creating…"
                                        }
                                    } else if renaming {
                                        "Rename"
                                    } else {
                                        "Create"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_remote_file(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn editor_conflict_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(conflict) = self.editor_conflict.as_ref() else {
            return div().into_any_element();
        };
        let name = Path::new(&conflict.path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&conflict.path)
            .to_string();
        let message = conflict
            .message
            .trim_start_matches("conflict:")
            .trim()
            .to_string();

        div()
            .id("editor-conflict-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_editor_conflict(cx)))
            .child(
                dialog_surface(p)
                    .id("editor-conflict")
                    .child(crate::effects::panel_probe(&self.overlay_backdrop, SurfaceGeometry::DIALOG, false))
                    .w(px(430.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .flex_shrink_0()
                                    .rounded(px(dunlin_ui::radii::PANEL))
                                    .bg(p.amber_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::TriangleAlert, 14.0, p.amber)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child("File changed elsewhere"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .line_height(relative(1.5))
                                            .text_color(p.muted_fg)
                                            .child(format!(
                                                "“{name}” changed since you opened it. {message} Reload the saved version or explicitly overwrite it with this buffer."
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(7.))
                            .child(
                                dunlin_ui::button("conflict-cancel").label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_editor_conflict(cx);
                                    })),
                            )
                            .child(
                                dunlin_ui::button("conflict-reload").label("Reload")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload_editor_conflict(window, cx);
                                    })),
                            )
                            .child(
                                dunlin_ui::button("conflict-overwrite").warning().label("Overwrite")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.overwrite_editor_conflict(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn project_settings_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let Some(index) = self.project_settings_target else {
            return div().into_any_element();
        };
        let Some(project) = self.projects.get(index).cloned() else {
            return div().into_any_element();
        };
        let selected_harness = project.settings.harness.clone();
        let selected_permission = project.settings.permission_mode;
        let focus = self
            .settings_ui
            .project_focus
            .as_ref()
            .unwrap_or(&self.focus_handle)
            .clone();
        let saving = self
            .project_runtime_draft
            .as_ref()
            .is_some_and(crate::project_runtime_settings::Draft::is_saving);
        let color_picker = if saving {
            dunlin_ui::settings::notice(
                "Saving project defaults… Other project choices are temporarily locked.",
                p,
            )
            .into_any_element()
        } else {
            self.project_color_picker(index, window, cx)
        };
        type ChipAction = Box<dyn Fn(&mut DunlinApp, &mut Context<DunlinApp>)>;

        let chip =
            |id: String, label: &str, active: bool, cx: &mut Context<Self>, action: ChipAction| {
                segment_button(id, label.to_owned(), active)
                    .disabled(saving)
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            };

        div()
            .id("project-settings-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| this.request_close_project_settings(window, cx)))
            .child(
                dialog_surface(p)
                    .id("project-settings-modal")
                    .role(Role::Dialog).aria_label("Project settings").tab_group()
                    .track_focus(&focus)
                    .child(crate::effects::panel_probe(&self.overlay_backdrop, SurfaceGeometry::DIALOG, false))
                    .w(px(620.)).max_w(relative(0.95))
                    .max_h(relative(0.85))
                    .overflow_y_scroll()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                Icon::new(IconName::FolderCog)
                                    .size(px(14.))
                                    .text_color(p.muted_fg),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.fg)
                                    .child(project.name.clone()),
                            )
                            .child(
                                icon_button("project-settings-close", IconName::X, "Close project settings")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_close_project_settings(window, cx);
                                    })),
                            ),
                    )
                    .child(div().text_size(px(11.0)).text_color(p.muted_fg).child(
                        "Overrides apply to this project. Defaults inherit from its group ancestry, then global settings.",
                    ))
                    .child(dunlin_ui::settings::notice(if project.is_remote() {
                        "Remote project. Appearance and client approval choices save on this Mac; the model/environment form is saved on the relay by an administrator."
                    } else {
                        "Project scope: this Mac. Appearance, agent and approval choices below save immediately. The separate model/environment form has its own Save and Cancel."
                    }, p))
                    .child(dunlin_ui::settings::section("Appearance and defaults · saved immediately", p))
                    .child(div().flex().items_center().gap(px(8.))
                        .child(crate::project_icons::render(self, &project, 32., &p))
                        .child(div().flex_1().text_size(px(12.)).child("Project icon"))
                        .child(chip("choose-project-icon".into(), "Choose image…", false, cx, Box::new(move |this, cx| this.choose_project_icon(index, cx))))
                         .child(chip("reset-project-icon".into(), "Automatic", project.settings.custom_icon.is_none() && project.settings.custom_color.is_none(), cx, Box::new(move |this, cx| this.reset_project_icon(index, cx)))))
                    .child(color_picker)
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child("HARNESS"),
                    )
                    .child({
                        let mut row = div().flex().flex_wrap().gap(px(4.));
                        row = row.child(chip(
                            "project-harness-default".into(),
                            "Inherit",
                            selected_harness.is_none(),
                            cx,
                            Box::new(move |this, cx| this.set_project_harness(index, None, cx)),
                        ));
                        for name in crate::harness_discovery::catalog_names()
                            .chain(std::iter::once("Custom ACP"))
                        {
                            if !self.harness_visible(name) { continue; }
                            let active = selected_harness.as_deref() == Some(name);
                            row = row.child(chip(
                                format!("project-harness-{name}"),
                                name,
                                active,
                                cx,
                                Box::new(move |this, cx| {
                                    this.set_project_harness(index, Some(name.to_string()), cx)
                                }),
                            ));
                        }
                        row
                    })
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child("PERMISSIONS"),
                    )
                    .child({
                        let mut row = div().flex().flex_wrap().gap(px(4.));
                        row = row.child(chip(
                            "project-permission-default".into(),
                            "Inherit",
                            selected_permission.is_none(),
                            cx,
                            Box::new(move |this, cx| this.set_project_permission(index, None, cx)),
                        ));
                        for mode in [
                            PermissionMode::Ask,
                            PermissionMode::ReadOnly,
                            PermissionMode::AutoApprove,
                        ] {
                            let active = selected_permission == Some(mode);
                            row = row.child(chip(
                                format!("project-permission-{}", mode.label()),
                                mode.label(),
                                active,
                                cx,
                                Box::new(move |this, cx| {
                                    this.set_project_permission(index, Some(mode), cx)
                                }),
                            ));
                        }
                        row
                    })
                    .when(!project.is_remote(), |view| view.child(match self.resolved_project_settings(&project) {
                        Ok(settings) => crate::group_settings::render_sources(self, &settings, cx),
                        Err(error) => div().text_size(px(11.)).text_color(p.red).child(error).into_any_element(),
                    }))
                    .child(dunlin_ui::settings::section("Model & environment · explicit Save / Cancel", p))
                    .child(crate::project_runtime_settings::render(self, index, cx))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg.opacity(0.8))
                            .child("Harness/model defaults apply to new conversations. Permission changes apply to subsequent requests without restarting."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                dunlin_ui::button("project-settings-reset").danger().outline().label("Reset overrides").disabled(saving)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.request_reset_project_settings(index, window, cx);
                                    })),
                            )
                            .child(div().flex_1())
                            .when(index == self.selected_project, |this| {
                                this.child(
                                     dunlin_ui::button("project-settings-new-session").label("New conversation")
                                          .on_click(cx.listener(|this, _, _, cx| {
                                             if this.project_runtime_draft.is_some() {
                                                 this.set_notice("Save or cancel model and environment changes before starting a conversation", cx);
                                                 return;
                                             }
                                            if this.chat_controls_busy() {
                                                this.set_notice("Wait for the current operation before starting a conversation", cx);
                                                return;
                                            }
                                            this.close_project_settings(cx);
                                            this.new_session(cx);
                                        })),
                                )
                            })
                            .child(
                                dunlin_ui::button("project-settings-done").primary().label("Done")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_close_project_settings(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn context_menu_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(state) = self.context_menu.clone() else {
            return div().into_any_element();
        };
        let viewport = window.viewport_size();
        let width = 228.0;
        let height = match &state.target {
            ContextTarget::Project(_) => 480.,
            ContextTarget::Session(_) => 260.,
            ContextTarget::Tab(_) => 170.,
        };
        let x = f32::from(state.position.x)
            .min(f32::from(viewport.width) - width - 8.0)
            .max(8.0);
        let y = f32::from(state.position.y)
            .min(f32::from(viewport.height) - height)
            .max(8.0);

        let item = |id: &'static str, label: &str, color: Hsla| {
            menu_button(id, label.to_owned())
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe(id)
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .when(color == p.red, |button| button.danger().outline())
                .when(color == p.amber, |button| button.warning().outline())
        };

        let menu = match state.target {
            ContextTarget::Project(index) => {
                let Some(project) = self.projects.get(index) else {
                    return div().into_any_element();
                };
                let missing = !project.is_remote() && !Path::new(&project.path).is_dir();
                let pinned = project.pinned;
                let trusted = project.trusted;
                let remote = project.is_remote();
                let reveal_path = project.path.clone();
                let path = project.path.clone();
                let name = project.name.clone();
                let mut menu =
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .px(px(8.))
                                .pt(px(3.))
                                .pb(px(4.))
                                .text_size(px(10.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(name.to_uppercase()),
                        )
                        .child(item("ctx-project-open", "Open project", p.fg).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.select_project(index, cx);
                            }),
                        ))
                        .child(item("ctx-project-group", "Move to group…", p.fg).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.context_menu = None;
                                this.project_group_editor = Some(index);
                                this.project_menu_open = false;
                                this.focus_handle.focus(window, cx);
                                cx.notify();
                            }),
                        ))
                        .child(
                            item("ctx-project-settings", "Project settings…", p.fg).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.open_project_settings(index, cx);
                                }),
                            ),
                        )
                        .child(
                            item(
                                "ctx-project-pin",
                                if pinned {
                                    "Unpin project"
                                } else {
                                    "Pin project"
                                },
                                p.fg,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.context_menu = None;
                                    this.toggle_project_pin_at(index, cx);
                                },
                            )),
                        );
                if missing {
                    menu = menu.child(
                        item("ctx-project-locate", "Locate folder…", p.amber).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.locate_project_at(index, cx);
                            }),
                        ),
                    );
                }
                menu = menu
                    .child(
                        item(
                            "ctx-project-trust",
                            if trusted {
                                "Untrust project"
                            } else {
                                "Trust project"
                            },
                            p.fg,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            this.toggle_project_trust_at(index, cx);
                        })),
                    )
                    .child(
                        item("ctx-project-reveal", "Reveal in Finder", p.fg).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.context_menu = None;
                                if remote {
                                    this.set_notice("Remote project · files live on the relay", cx);
                                } else {
                                    cx.reveal_path(Path::new(&reveal_path));
                                    cx.notify();
                                }
                            },
                        )),
                    )
                    .child(
                        item("ctx-project-path", "Copy path", p.fg).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.context_menu = None;
                                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                                this.set_notice("Copied the project path", cx);
                            },
                        )),
                    );
                menu =
                    menu.child(div().h(px(1.)).my(px(2.)).bg(p.border))
                        .child(item("ctx-new-group", "New project group…", p.fg).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.context_menu = None;
                                this.open_new_group_prompt(None, window, cx);
                            }),
                        ))
                        .child(
                            item("ctx-open-folders", "Open local folders…", p.fg).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.context_menu = None;
                                    this.open_folder(cx);
                                }),
                            ),
                        )
                        .child(
                            item("ctx-add-remote", "Add remote project…", p.fg).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.context_menu = None;
                                    this.open_remote_picker(cx);
                                }),
                            ),
                        );
                menu.child(div().h(px(1.)).my(px(2.)).bg(p.border)).child(
                    item("ctx-project-remove", "Remove project…", p.red).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.context_menu = None;
                            this.request_delete_project(index, cx);
                        },
                    )),
                )
            }
            ContextTarget::Session(id) => {
                let Some(session) = self.all_sessions.iter().find(|session| session.id == id)
                else {
                    return div().into_any_element();
                };
                let open_id = session.id.clone();
                let new_id = session.id.clone();
                let delete_id = session.id.clone();
                let title = session.title.clone();
                let remote = session.origin.is_remote();
                let local_index = self.sessions.iter().position(|s| s.id == session.id);
                let mut menu =
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .px(px(8.))
                                .pt(px(3.))
                                .pb(px(4.))
                                .text_size(px(10.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(title.to_uppercase()),
                        )
                        .child(item("ctx-session-open", "Open session", p.fg).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.context_menu = None;
                                if this.open_session_id(&open_id, cx) {
                                    this.show_chat_tab(window, cx);
                                }
                            }),
                        ))
                        .child(
                            item("ctx-session-new", "New session", p.fg).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.context_menu = None;
                                    if this.open_session_id(&new_id, cx)
                                        && !this.remote_sessions_loading
                                    {
                                        this.new_session(cx);
                                    }
                                },
                            )),
                        );
                if remote && let Some(index) = local_index {
                    menu = menu.child(
                        item("ctx-session-takeover", "Take over session", p.fg).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.takeover_session_at(index, cx);
                            }),
                        ),
                    );
                }
                menu.child(div().h(px(1.)).my(px(2.)).bg(p.border)).child(
                    item("ctx-session-delete", "Delete session…", p.red).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.context_menu = None;
                            this.request_delete_session_id(&delete_id, cx);
                        },
                    )),
                )
            }
            ContextTarget::Tab(id) => {
                let Some(tab) = self.workbench.tabs.iter().find(|tab| tab.id == id) else {
                    return div().into_any_element();
                };
                let name = tab.inspector.name.clone();
                let locked = tab.locked;
                let close_id = id.clone();
                let lock_id = id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .px(px(8.))
                            .pt(px(3.))
                            .pb(px(4.))
                            .text_size(px(10.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .truncate()
                            .child(name.to_uppercase()),
                    )
                    .child(
                        item(
                            "ctx-tab-lock",
                            if locked { "Unlock tab" } else { "Lock tab" },
                            p.fg,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            if let Some(tab) =
                                this.workbench.tabs.iter_mut().find(|tab| tab.id == lock_id)
                            {
                                tab.locked = !tab.locked;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        item("ctx-tab-close", "Close tab", p.fg).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.context_menu = None;
                                this.request_close_tab(&close_id, window, cx);
                            },
                        )),
                    )
            }
        };

        div()
            .id("context-menu-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_click(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
            .child(
                div()
                    .id("context-menu")
                    .popover_style(cx)
                    .occlude()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(width))
                    .max_h(px((f32::from(viewport.height) - y - 8.).max(1.)))
                    .overflow_y_scroll()
                    .p(px(5.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(menu.flex_shrink_0()),
            )
            .into_any_element()
    }

    pub(super) fn toast_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(toast) = self.toast.as_ref() else {
            return div().into_any_element();
        };
        let message = toast.message.clone();
        let action_label = toast.action_label.clone();

        div()
            .absolute()
            .top(px(54.))
            .right(px(14.))
            .w(px(320.))
            .rounded(px(dunlin_ui::radii::DRAG_PREVIEW))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.card)
            .p(px(9.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .size(px(22.))
                    .flex_shrink_0()
                    .rounded(px(dunlin_ui::radii::ROW))
                    .bg(p.green_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::Check).size(px(12.)).text_color(p.green)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(11.5))
                    .text_color(p.fg)
                    .child(message),
            )
            .child(
                dunlin_ui::button("toast-action")
                    .primary()
                    .label(action_label)
                    .on_click(cx.listener(|this, _, _, cx| this.apply_toast(cx))),
            )
            .child(
                icon_button("toast-dismiss", IconName::X, "Dismiss notification")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_toast(cx))),
            )
            .into_any_element()
    }

    fn group_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(prompt) = self.group_prompt.as_ref() else {
            return div().into_any_element();
        };
        let editing = prompt.id.clone();
        let title = if editing.is_some() {
            "Rename group"
        } else {
            "New project group"
        };
        let hint = if editing.is_some() {
            "Deleting promotes direct projects and subgroups one level. Files and existing conversations stay unchanged."
        } else {
            "Create a logical project group. Files stay in place. Right-click a group to add subgroups."
        };
        let delete_id = editing.clone();

        div()
            .id("group-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| this.cancel_group_prompt(cx)))
            .child(
                dialog_surface(p)
                    .id("group-modal")
                    .child(crate::effects::panel_probe(
                        &self.overlay_backdrop,
                        SurfaceGeometry::DIALOG,
                        false,
                    ))
                    .w(px(380.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                Icon::new(IconName::FolderTree)
                                    .size(px(14.))
                                    .text_color(p.muted_fg),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.fg)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .line_height(relative(1.5))
                            .text_color(p.muted_fg)
                            .child(hint),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "Location: {}",
                                prompt
                                    .parent_id
                                    .as_ref()
                                    .map(|id| crate::project_tree::group_path(
                                        &self.project_groups,
                                        id
                                    ))
                                    .unwrap_or_else(|| "Top level".into())
                            )),
                    )
                    .child(dunlin_ui::input(&self.group_input).aria_label("Project group name"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .when_some(delete_id, |this, id| {
                                this.child(
                                    dunlin_ui::button("group-delete")
                                        .danger()
                                        .outline()
                                        .label("Delete group")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_project_group(&id, cx);
                                        })),
                                )
                            })
                            .child(div().flex_1())
                            .child(dunlin_ui::button("group-cancel").label("Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.cancel_group_prompt(cx);
                                }),
                            ))
                            .child(
                                dunlin_ui::button("group-save")
                                    .primary()
                                    .label(if editing.is_some() {
                                        "Save"
                                    } else {
                                        "Create group"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_group_prompt(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn delete_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(pending) = self.pending_delete.clone() else {
            return div().into_any_element();
        };
        let (title, body, action) = match &pending {
            PendingDelete::Session { title, .. } => (
                "Delete session?".to_string(),
                format!(
                    "“{title}” and its transcript will be removed from this Mac. This cannot be undone."
                ),
                "Delete session",
            ),
            PendingDelete::Project { name, .. } => (
                "Remove project?".to_string(),
                format!(
                    "“{name}” will be removed from Dunlin. The folder stays on disk, but its sessions and transcripts will be deleted."
                ),
                "Remove project",
            ),
            PendingDelete::History { count } => (
                "Clear conversation history?".to_string(),
                format!(
                    "{count} sessions and their transcripts will be deleted from this Mac. Projects and settings stay. This cannot be undone."
                ),
                "Clear history",
            ),
        };

        div()
            .id("delete-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(20.))
            .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx)))
            .child(
                dialog_surface(p)
                    .id("delete-dialog")
                    .relative()
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe("delete-dialog")
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .child(crate::effects::panel_probe(
                        &self.overlay_backdrop,
                        SurfaceGeometry::DIALOG,
                        true,
                    ))
                    .w(px(392.))
                    .max_w_full()
                    .max_h(relative(0.85))
                    .overflow_y_scroll()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .flex_shrink_0()
                                    .flex_shrink_0()
                                    .rounded(px(dunlin_ui::radii::PANEL))
                                    .bg(p.red_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Trash, 14.0, p.red)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_normal()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.fg)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .relative()
                                            .children({
                                                #[cfg(debug_assertions)]
                                                {
                                                    crate::workspace_ui_smoke::probe(
                                                        "delete-message",
                                                    )
                                                }
                                                #[cfg(not(debug_assertions))]
                                                {
                                                    None::<AnyElement>
                                                }
                                            })
                                            .text_size(px(12.0))
                                            .line_height(relative(1.5))
                                            .text_color(p.muted_fg)
                                            .child(body),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .flex_wrap()
                            .gap(px(7.))
                            .child(
                                dunlin_ui::button("delete-cancel")
                                    .relative()
                                    .label("Cancel")
                                    .children({
                                        #[cfg(debug_assertions)]
                                        {
                                            crate::workspace_ui_smoke::probe("delete-cancel")
                                        }
                                        #[cfg(not(debug_assertions))]
                                        {
                                            None::<AnyElement>
                                        }
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx))),
                            )
                            .child(
                                dunlin_ui::button("delete-confirm")
                                    .relative()
                                    .danger()
                                    .label(action)
                                    .children({
                                        #[cfg(debug_assertions)]
                                        {
                                            crate::workspace_ui_smoke::probe("delete-confirm")
                                        }
                                        #[cfg(not(debug_assertions))]
                                        {
                                            None::<AnyElement>
                                        }
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.confirm_delete(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn inspector_panel(&self, _inspector: &Inspector, cx: &mut Context<Self>) -> AnyElement {
        if self.active_file_tab().is_some_and(|tab| {
            tab.kind == dunlin_workspace::preview::PreviewKind::Text
                && crate::markdown_editor::supports(&tab.inspector.path)
        }) {
            return crate::markdown_editor::render(self, cx);
        }
        if self
            .active_file_tab()
            .is_some_and(|t| t.kind != dunlin_workspace::preview::PreviewKind::Text)
        {
            return self.preview_panel(cx);
        }
        let readonly = self.editor_readonly;

        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .overflow_hidden()
                            // GPUI overflow masks are rectangular; keep gutter/selection paint inside the curved edges.
                            .pl(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                            .when(!self.editor_minimap, |body| {
                                body.pr(px(dunlin_ui::scale::WORKSPACE_RADIUS))
                            })
                            .child(
                                Editor::new(&self.editor)
                                    .text_size(crate::workspace_text::size(self.code_font_size, cx))
                                    .appearance(false)
                                    .bordered(false)
                                    .readonly(
                                        readonly
                                            || self.explorer.busy
                                            || self.ai_commit_locks_project(),
                                    )
                                    .h_full(),
                            )
                            .relative()
                            .child(crate::inline_blame::overlay(self, cx)),
                    )
                    .when(self.editor_minimap, |body| {
                        body.child(crate::minimap::render(self, true, cx))
                    }),
            )
            .when_some(self.retained_buffer_banner(cx), |root, banner| {
                root.child(banner)
            })
            .when_some(crate::inline_blame::status(self, cx), |root, status| {
                root.child(status)
            })
            .into_any_element()
    }

    pub(super) fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let session = self.session();
        let branch = if self.git.branch.trim().is_empty() {
            self.project()
                .branch
                .clone()
                .unwrap_or_else(|| "Branches…".into())
        } else {
            self.git.branch.clone()
        };
        let change_count = self.git.changes.len();
        let project = self.project();
        let remote = self
            .repo_summaries
            .values
            .get(&project.id)
            .filter(|summary| summary.path == project.path)
            .map(|summary| &summary.activity);
        let upstream = remote.and_then(|activity| {
            activity
                .remote_name
                .as_ref()
                .and_then(|_| activity.upstream.clone())
        });
        let ahead = remote.and_then(|activity| activity.ahead);
        let behind = remote.and_then(|activity| activity.behind);
        let chat = matches!(
            self.current_workspace_tab(),
            Some(crate::workspace_panes::TabKey::Chat(_))
        ) && self.view == MainView::Chat;
        let repository = self
            .git
            .repository
            .as_ref()
            .filter(|link| self.project().trusted && link.validated())
            .cloned();
        let hosting_icon = match repository.as_ref().map(|link| &link.provider) {
            Some(dunlin_workspace::git::hosting::Provider::GitHub) => IconName::Github,
            Some(dunlin_workspace::git::hosting::Provider::GitLab) => IconName::GitFork,
            None => IconName::Link,
        };
        let hosting_label = repository
            .as_ref()
            .map(|link| format!("Open {}", link.url))
            .unwrap_or_else(|| "No supported repository host".into());
        let owner = cx.entity().downgrade();
        let usage = dunlin_ui::component::popover::Popover::new("status-usage-popover")
            .when(self.has_modal_overlay(), |popover| popover.open(false))
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(
                dunlin_ui::button("status-usage")
                    .relative()
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe("status-usage")
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                MorphText::new(
                                    "status-session-cost",
                                    format!("${:.2}", session.cost_usd),
                                )
                                .numeric()
                                .scope(session.id.clone())
                                .reduce_motion(self.reduce_motion),
                            )
                            .child(" · ")
                            .child(
                                MorphText::new(
                                    "status-account-allowance",
                                    crate::usage::account_status(&self.active_usages()),
                                )
                                .numeric()
                                .scope(format!("{:?}", self.usage_target()))
                                .reduce_motion(self.reduce_motion),
                            ),
                    )
                    .accessibility_label(format!(
                        "Session cost ${:.2}. {}. Open account limits",
                        session.cost_usd,
                        crate::usage::account_status(&self.active_usages())
                    ))
                    .tooltip("Session cost and account allowance · opens current usage details"),
            )
            .on_open_change({
                let owner = cx.entity().downgrade();
                move |open, _, cx| {
                    if *open {
                        let _ = owner.update(cx, |this, cx| this.ensure_usage(cx));
                    }
                }
            })
            .content(move |_, window, cx| {
                owner
                    .update(cx, |this, cx| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .px(px(10.))
                                    .text_size(px(12.))
                                    .child(format!("Session · ${:.2}", this.session().cost_usd)),
                            )
                            .child(views::sidebar::account_usage_panel(this, window, cx))
                            .into_any_element()
                    })
                    .unwrap_or_else(|_| div().into_any_element())
            });
        div()
            .h(px(crate::branch_picker::STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .overflow_hidden()
            .px(px(crate::branch_picker::STATUS_BAR_INSET))
            .flex()
            .items_center()
            .gap(px(12.))
            .text_size(px(12.5))
            .text_color(p.fg)
            .child(
                dunlin_ui::button("repository-host-link")
                    .ghost()
                    .size(px(24.))
                    .p(px(0.))
                    .child(Icon::new(hosting_icon).size(px(15.)))
                    .accessibility_label(hosting_label.clone())
                    .tooltip(hosting_label)
                    .disabled(repository.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(link) = &repository {
                            cx.open_url(&link.url);
                        }
                    }),
            )
            .child(crate::branch_picker::status_badge(
                self,
                &self.project().id,
                &branch,
                cx,
            ))
            .when(!chat, |view| {
                let mut row = view;
                if let Some(upstream) = upstream.clone() {
                    let incoming = behind.unwrap_or(0);
                    let outgoing = ahead.unwrap_or(0);
                    let diverged = incoming > 0 && outgoing > 0;
                    let label = if incoming > 0 {
                        format!("Pull {incoming}")
                    } else if behind == Some(0) {
                        "Up to date".into()
                    } else {
                        "Fetch".into()
                    };
                    let tooltip = match (behind, ahead) {
                        (Some(0), Some(0)) => {
                            format!("Up to date with {upstream} · click to fetch again")
                        }
                        (Some(0), Some(ahead)) => format!(
                            "No incoming commits · {ahead} local {} ahead of {upstream} · click to fetch",
                            if ahead == 1 { "commit" } else { "commits" }
                        ),
                        (Some(behind), Some(ahead)) if ahead > 0 => format!(
                            "Diverged: {behind} behind and {ahead} ahead of {upstream} · click to fetch; fast-forward may be blocked"
                        ),
                        (Some(behind), _) => format!(
                            "{behind} remote {} behind {upstream} · click to fetch and fast-forward",
                            if behind == 1 { "commit" } else { "commits" }
                        ),
                        _ => format!("Fetch {upstream} to check for incoming commits"),
                    };
                    row = row.child(
                        segment_button("status-remote-pull", label, false)
                            .with_size(dunlin_ui::component::Size::Medium)
                            .h(px(28.))
                            .icon(IconName::CloudDownload)
                            .text_color(if diverged {
                                p.amber
                            } else if incoming > 0 {
                                p.blue
                            } else {
                                p.muted_fg
                            })
                            .accessibility_label(format!("Pull remote changes from {upstream}"))
                            .tooltip(tooltip)
                            .on_click(cx.listener(|this, _, _, cx| this.git_pull(cx))),
                    );
                }
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_sm()
                        .child(icon(IconName::FileDiff, 10.5, p.fg))
                        .child(MorphText::new("status-git-changes", change_count.to_string()).numeric()
                            .scope(project.id.clone()).reduce_motion(self.reduce_motion))
                        .child(if change_count == 1 { "change" } else { "changes" }),
                )
                .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            })
            .child(div().flex_1())
            .child({
                let panel = self.bottom_panel;
                let active = panel == crate::app::BottomPanel::Terminal;
                segment_button("statusbar-terminal", "Terminal", active)
                    .with_size(dunlin_ui::component::Size::Medium)
                    .h(px(28.))
                    .accessibility_label("Toggle terminal panel")
                    .icon(IconName::Terminal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_bottom_panel(crate::app::BottomPanel::Terminal, window, cx)
                    }))
            })
            .child({
                let active = self.bottom_panel == crate::app::BottomPanel::Run;
                segment_button("statusbar-run", "Run", active)
                    .with_size(dunlin_ui::component::Size::Medium)
                    .h(px(28.))
                    .accessibility_label("Toggle Run panel")
                    .icon(IconName::SquareTerminal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_bottom_panel(crate::app::BottomPanel::Run, window, cx)
                    }))
            })
            .child({
                let active = self.bottom_panel == crate::app::BottomPanel::Secrets;
                segment_button("statusbar-secrets", "Secrets", active)
                    .with_size(dunlin_ui::component::Size::Medium)
                    .h(px(28.))
                    .accessibility_label("Toggle Secrets panel")
                    .icon(IconName::KeyRound)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_bottom_panel(crate::app::BottomPanel::Secrets, window, cx)
                    }))
            })
            .child(usage)
            .when(self.harness.usage.is_some() || session.tokens > 0, |bar| {
                bar.child(
                    div()
                        .text_color(p.muted_fg)
                        .child(MorphText::new("status-context", context_status_label(self.harness.usage, session.tokens))
                            .numeric().scope(session.id.clone()).reduce_motion(self.reduce_motion)),
                )
            })
            .into_any_element()
    }

    pub(crate) fn has_modal_overlay(&self) -> bool {
        self.settings_open || self.navigation_blocked()
    }
    pub(crate) fn navigation_blocked(&self) -> bool {
        (self.settings_open && self.settings_have_draft())
            || self.group_cleanup.busy
            || self.remote_connection.open
            || self.pending_delete.is_some()
            || self.pending_git_action.is_some()
            || self.diff_view.is_some()
            || self.new_file_prompt
            || self.pending_editor.is_some()
            || self.editor_conflict.is_some()
            || self.group_prompt.is_some()
            || self.project_group_editor.is_some()
            || self.project_import.open
            || self.project_settings_target.is_some()
            || self.group_settings_draft.is_some()
            || self.provider_auth.is_some()
            || self.factory_resetting
    }
    pub(crate) fn has_overlay(&self) -> bool {
        self.has_modal_overlay()
            || self.command_open
            || self.remote_picker.is_some()
            || self.context_menu.is_some()
            || self.branch_ui.is_open()
            || self.project_menu_open
    }
    pub(crate) fn overlay_focus_handle(&self, cx: &App) -> FocusHandle {
        if self.remote_connection.open {
            return if self.remote_connection.ssh {
                self.terminal
                    .ssh_target
                    .as_ref()
                    .map(|input| input.read(cx).focus_handle(cx))
                    .unwrap_or_else(|| self.focus_handle.clone())
            } else {
                self.remote_url_input.read(cx).focus_handle(cx)
            };
        }
        if self.project_import.open
            && let Some(focus) = self.project_import.focus(cx)
        {
            return focus;
        }
        if let Some(draft) = &self.group_settings_draft {
            return draft.focus_handle(cx);
        }
        if self.project_settings_target.is_some()
            && let Some(draft) = &self.project_runtime_draft
        {
            return draft.focus_handle(cx);
        }
        if self.project_settings_target.is_some() {
            return self
                .settings_ui
                .project_focus
                .clone()
                .unwrap_or_else(|| self.focus_handle.clone());
        }
        if let Some(focus) = self.branch_ui.input_focus(cx) {
            return focus;
        }
        if self.settings_open {
            if let Some(draft) = &self.vault.ui.draft {
                return draft.label.read(cx).focus_handle(cx);
            }
            if let Some(draft) = &self.vault.ui.recipe_draft {
                return draft.name.read(cx).focus_handle(cx);
            }
            if let Some(draft) = &self.vault.ui.operation_draft {
                return draft.name.read(cx).focus_handle(cx);
            }
            if let Some(focus) = self.skills_ui.draft_focus(cx) {
                return focus;
            }
            if let Some(search) = &self.settings_ui.search {
                return search.read(cx).focus_handle(cx);
            }
        }
        if self.command_open {
            return self.command_query.read(cx).focus_handle(cx);
        }
        if self.new_file_prompt {
            return self.new_file_input.read(cx).focus_handle(cx);
        }
        if self.group_prompt.is_some() {
            return self.group_input.read(cx).focus_handle(cx);
        }
        self.focus_handle.clone()
    }
    pub(super) fn render_overlay_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.has_modal_overlay() {
            self.project_menu_open = false;
        }
        let settings = self
            .settings_open
            .then(|| views::settings::render(self, window, cx));
        let palette = self
            .command_open
            .then(|| views::palette::render(self, window, cx));
        let confirm = self.pending_delete.as_ref().map(|_| self.delete_modal(cx));
        let git_confirm = self
            .pending_git_action
            .as_ref()
            .map(|_| crate::git_actions::render(self, cx));
        let diff = self.diff_view.as_ref().map(|_| self.diff_modal(cx));
        let remote = self
            .remote_picker
            .as_ref()
            .map(|_| self.remote_picker_modal(cx));
        let new_file = self.new_file_prompt.then(|| self.new_file_modal(cx));
        let editor = self.pending_editor.as_ref().map(|_| self.editor_prompt(cx));
        let conflict = self
            .editor_conflict
            .as_ref()
            .map(|_| self.editor_conflict_modal(cx));
        let group = self.group_prompt.as_ref().map(|_| self.group_modal(cx));
        let toast = self.toast.as_ref().map(|_| self.toast_overlay(cx));
        let context = self
            .context_menu
            .as_ref()
            .map(|_| self.context_menu_overlay(window, cx));
        let project = self
            .project_settings_target
            .map(|_| self.project_settings_modal(window, cx));
        let group_settings = self
            .group_settings_draft
            .as_ref()
            .map(|_| crate::group_settings::render(self, cx));
        let auth = self
            .provider_auth
            .as_ref()
            .map(|_| crate::provider_auth::render(self, cx));
        let branches = self
            .branch_ui
            .is_open()
            .then(|| crate::branch_picker::render(self, window, cx));
        let project_menu = self
            .project_menu_open
            .then(|| crate::views::sidebar::project_menu(self, cx));
        let import = self
            .project_import
            .open
            .then(|| crate::project_import::render(self, window, cx));
        let connection = self
            .remote_connection
            .open
            .then(|| crate::remote_connection::render(self, cx));
        let move_group = self.project_group_editor.and_then(|index| {
            let project = self.projects.get(index)?;
            Some(
                div()
                    .id("project-group-overlay")
                    .absolute()
                    .inset_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_group_editor = None;
                        cx.notify();
                    }))
                    .child(
                        dialog_surface(dunlin_ui::palette(cx))
                            .id("project-group-dialog")
                            .w(px(520.))
                            .max_w(relative(0.95))
                            .max_h(relative(0.9))
                            .children({
                                #[cfg(debug_assertions)]
                                {
                                    crate::workspace_ui_smoke::probe("project-group-dialog")
                                }
                                #[cfg(not(debug_assertions))]
                                {
                                    None::<AnyElement>
                                }
                            })
                            .child(crate::effects::panel_probe(
                                &self.overlay_backdrop,
                                SurfaceGeometry::DIALOG,
                                false,
                            ))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Move {} to group", project.name)),
                            )
                            .child(div().id("project-group-options").overflow_y_scroll().child(
                                views::sidebar::project_group_picker(
                                    self,
                                    index,
                                    cx,
                                    &dunlin_ui::palette(cx),
                                ),
                            ))
                            .child(
                                dunlin_ui::button("project-group-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.project_group_editor = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
            )
        });
        div()
            .absolute()
            .inset_0()
            .size_full()
            .children(settings)
            .children(palette)
            .children(confirm)
            .children(diff)
            .children(remote)
            .children(new_file)
            .children(editor)
            .children(conflict)
            .children(group)
            .children(toast)
            .children(context)
            .children(project)
            .children(group_settings)
            .children(auth)
            .children(branches)
            .children(project_menu)
            .children(git_confirm)
            .children(import)
            .children(connection)
            .children(move_group)
            .when(self.factory_resetting, |root| {
                root.child(
                    div()
                        .id("factory-reset-progress")
                        .absolute()
                        .inset_0()
                        .size_full()
                        .bg(dunlin_ui::palette(cx).card.opacity(0.95))
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child("Factory reset · waiting for confirmation or clearing local data…"),
                )
            })
            .into_any_element()
    }
    pub(crate) fn render_overlay_plane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let content = self.render_overlay_content(window, cx);
        div()
            .relative()
            .size_full()
            .text_color(p.fg)
            .font_family(dunlin_ui::theme::ui_font())
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::on_key_down))
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Back, window, cx| {
                    this.navigate_history(false, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::native_menu::Forward, window, cx| {
                    this.navigate_history(true, window, cx);
                }),
            )
            .when(!self.factory_resetting, |root| {
                root.on_action(
                    cx.listener(|this, _: &crate::native_menu::Settings, _, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.branch_ui.dismiss();
                            this.settings_open = true;
                            this.settings_tab = SettingsTab::General;
                            cx.notify();
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::OpenFolders, _, cx| {
                        this.open_folder(cx)
                    }),
                )
                .on_action(cx.listener(|this, _: &crate::native_menu::Home, _, cx| {
                    if this.dismiss_history_for_menu(cx) {
                        this.section = SidebarSection::Dashboard;
                        this.settings_open = false;
                        cx.notify();
                    }
                }))
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::Projects, _, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.section = SidebarSection::Projects;
                            cx.notify();
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::ToggleSidebar, _, cx| {
                        this.sidebar_collapsed = !this.sidebar_collapsed;
                        this.persist_settings(cx);
                        cx.notify();
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::Commands, window, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.open_palette(window, cx);
                        }
                    }),
                )
                .when(!self.projects.is_empty(), |root| {
                    root.on_action(cx.listener(
                        |this, _: &crate::native_menu::NewSession, _, cx| {
                            if this.dismiss_history_for_menu(cx) {
                                this.new_session(cx);
                            }
                        },
                    ))
                    .on_action(cx.listener(|this, _: &crate::native_menu::SaveAll, _, cx| {
                        this.save_all_tabs(cx);
                    }))
                    .on_action(cx.listener(
                        |this, _: &crate::native_menu::ProjectSettings, _, cx| {
                            if this.dismiss_history_for_menu(cx) {
                                this.open_project_settings(this.selected_project, cx);
                            }
                        },
                    ))
                    .on_action(cx.listener(
                        |this, _: &crate::native_menu::SwitchBranch, window, cx| {
                            if this.dismiss_history_for_menu(cx) {
                                this.open_branch_picker(&this.project().id.clone(), window, cx);
                            }
                        },
                    ))
                    .on_action(cx.listener(
                        |this, _: &crate::native_menu::RefreshGit, _, cx| {
                            this.refresh_git_status(cx)
                        },
                    ))
                })
                .when(
                    !self.projects.is_empty() && self.project().trusted,
                    |root| {
                        root.on_action(cx.listener(
                            |this, _: &crate::native_menu::GitHistory, window, cx| {
                                this.open_git_history(window, cx)
                            },
                        ))
                    },
                )
            })
            .child(self.overlay_backdrop.begin())
            .child(content)
            .child(self.overlay_backdrop.finish())
            .into_any_element()
    }
}

fn onboarding_primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    _p: &Palette,
) -> Button {
    dunlin_ui::button(id).primary().large().label(label)
}

fn onboarding_secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    _p: &Palette,
) -> Button {
    dunlin_ui::button(id).label(label)
}

fn onboarding_text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    _p: &Palette,
) -> Button {
    dunlin_ui::button(id).ghost().label(label)
}
