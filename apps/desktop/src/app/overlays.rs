use super::*;

impl DunlinApp {
    pub(super) fn rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let mut rail = div()
            .w(px(52.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .py(px(9.))
            .gap(px(3.))
            .bg(p.panel)
            .border_r_1()
            .border_color(p.border);

        let items = [
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
            (
                SidebarSection::Sessions,
                IconName::Clock3,
                self.sessions.len(),
                false,
            ),
        ];

        for (section, icon_name, count, has_badge) in items {
            let active = self.section == section && !self.settings_open;
            let color = if active { p.fg } else { p.muted_fg };
            let mut button = div()
                .id(("rail", section as usize))
                .relative()
                .size(px(36.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    this.sidebar_collapsed = false;
                    this.settings_open = false;
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(icon(icon_name, 17.0, color));

            if active {
                button = button.child(
                    div()
                        .absolute()
                        .left(px(-8.))
                        .h(px(16.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(p.fg),
                );
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
                div()
                    .id("rail-usage")
                    .size(px(34.))
                    .rounded(px(8.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(2.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = true;
                        this.settings_tab = SettingsTab::Usage;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(2.))
                            .h(px(13.))
                            .child(div().w(px(3.)).h(px(6.)).rounded(px(1.)).bg(p.violet))
                            .child(div().w(px(3.)).h(px(10.)).rounded(px(1.)).bg(p.blue))
                            .child(div().w(px(3.)).h(px(13.)).rounded(px(1.)).bg(p.teal)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child(
                                self.active_usages()
                                    .first()
                                    .and_then(|usage| usage.primary())
                                    .map(|window| format!("{:.0}%", window.used_percent))
                                    .unwrap_or_else(|| "—".into()),
                            ),
                    ),
            )
            .child(
                div()
                    .id("rail-settings")
                    .size(px(36.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(self.settings_open, |this| this.bg(p.accent))
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = !this.settings_open;
                        this.settings_tab = SettingsTab::Harnesses;
                        cx.notify();
                    }))
                    .child(icon(
                        IconName::Settings,
                        16.5,
                        if self.settings_open { p.fg } else { p.muted_fg },
                    )),
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
        self.session_menu_open = false;
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
                        this.pending_remote_session = Some(PendingRemoteSession::Load {
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

    fn session_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);

        fn item(
            id: &'static str,
            icon_name: IconName,
            label: &'static str,
            hint: Option<&'static str>,
            danger: bool,
            cx: &mut Context<DunlinApp>,
            action: fn(&mut DunlinApp, &mut Window, &mut Context<DunlinApp>),
        ) -> AnyElement {
            let p = palette(cx);
            div()
                .id(id)
                .h(px(30.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(if danger { p.red_soft } else { p.hover }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    action(this, window, cx);
                }))
                .child(icon(
                    icon_name,
                    12.5,
                    if danger { p.red } else { p.muted_fg },
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(if danger { p.red } else { p.fg })
                        .child(label),
                )
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(hint)
                }))
                .into_any_element()
        }

        let takeover = self.session().origin.is_remote().then(|| {
            item(
                "session-menu-takeover",
                IconName::Cloud,
                "Take over session",
                None,
                false,
                cx,
                |this, _, cx| {
                    this.session_menu_open = false;
                    this.takeover_current_session(cx);
                },
            )
        });

        div()
            .id("session-menu-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_click(cx.listener(|this, _, _, cx| {
                this.session_menu_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("session-menu")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 10.))
                    .absolute()
                    .top(px(42.))
                    .right(px(14.))
                    .w(px(190.))
                    .p(px(5.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(item(
                        "session-menu-new",
                        IconName::Plus,
                        "New session",
                        self.show_shortcut_hints.then_some("⌘N"),
                        false,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            this.new_session(cx);
                        },
                    ))
                    .child(item(
                        "session-menu-copy",
                        IconName::Copy,
                        "Copy transcript",
                        Some("MD"),
                        false,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            if this.harness.transcript.is_empty() {
                                this.set_notice("Nothing to copy yet", cx);
                                return;
                            }
                            let markdown = harness::transcript_markdown(&this.harness.transcript);
                            cx.write_to_clipboard(ClipboardItem::new_string(markdown));
                            this.set_notice("Copied the transcript as Markdown", cx);
                        },
                    ))
                    .children(takeover)
                    .child(div().h(px(1.)).my(px(3.)).bg(p.border))
                    .child(item(
                        "session-menu-delete",
                        IconName::Trash,
                        "Delete session…",
                        None,
                        true,
                        cx,
                        |this, _, cx| {
                            this.session_menu_open = false;
                            this.request_delete_session(this.selected_session, cx);
                        },
                    )),
            )
            .into_any_element()
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
                .filter(|detected| {
                    crate::harness::command_available(&crate::harness::harness_command(
                        detected.name,
                        "",
                    ))
                })
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
            "Dunlin runs local ACP adapters. Pick the agent new conversations should use."
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
            .min_w(px(0.))
            .items_center()
            .justify_center()
            .gap(px(18.))
            .child(
                div()
                    .size(px(38.))
                    .rounded(px(10.))
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
                    crate::harness::command_available(&crate::harness::harness_command(
                        detected.name,
                        "",
                    ))
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut rows = div().w(px(460.)).flex().flex_col().gap(px(6.));
            for detected in available {
                let name = detected.name;
                let selected = self.default_harness == name;
                let name_owned = name.to_string();
                let name_for_button = name_owned.clone();
                let row = div()
                    .id(format!("onboarding-harness:{name}"))
                    .h(px(52.))
                    .w_full()
                    .px(px(12.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if selected { p.border_strong } else { p.border })
                    .bg(p.card)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
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
                                    .child(dot(p.green, 4.0))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(p.muted_fg)
                                            .child("Detected"),
                                    ),
                            ),
                    )
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
                    } else {
                        onboarding_secondary_button(
                            format!("onboarding-harness-use:{name}"),
                            "Set default",
                            &p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.default_harness = name_for_button.clone();
                            this.persist_settings(cx);
                            cx.notify();
                        }))
                        .into_any_element()
                    }));
                rows = rows.child(row);
            }
            body = body.child(rows);
            if self.harness_discovery.iter().all(|detected| {
                !crate::harness::command_available(&crate::harness::harness_command(
                    detected.name,
                    "",
                ))
            }) {
                body = body.child(
                    div()
                        .max_w(px(440.))
                        .text_center()
                        .text_size(px(11.0))
                        .line_height(relative(1.45))
                        .text_color(p.muted_fg)
                        .child("No ACP adapter was detected. You can install one later in Settings › Harnesses."),
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
            body = body.child(onboarding_permission_card(cx, &p)).child(
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
                        .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
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
        if self.terminal_resize.take().is_some() || self.terminal_drag {
            self.terminal_drag = false;
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
                if self.terminal.tabs.is_empty()
                    && !self.project().is_remote()
                    && self.project().trusted
                    && let Err(error) = self.open_terminal_tab(cx)
                {
                    self.terminal.error = Some(error);
                }
                self.terminal.focus.focus(window, cx);
            }
            BottomPanel::Run => self.ensure_run_detection(cx),
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
        let mut header = div()
            .h(px(30.))
            .flex_shrink_0()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .border_t_1()
            .border_b_1()
            .border_color(p.border)
            .bg(p.subtle)
            .child(
                div()
                    .id("terminal-dock-grip")
                    .aria_label("Drag to move the terminal panel to the left, right or bottom edge")
                    .h(px(20.))
                    .px(px(3.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .tooltip(|window, cx| {
                        Tooltip::new("Drag the panel to the bottom, left or right edge")
                            .build(window, cx)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.terminal_drag = true;
                            cx.notify();
                        }),
                    )
                    .child("⠿"),
            );
        for (panel, label) in [
            (BottomPanel::Terminal, "Terminal"),
            (BottomPanel::Run, "Run"),
        ] {
            let active = self.bottom_panel == panel;
            header = header.child(
                div()
                    .id(format!("bottom-panel:{}", panel as usize))
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(p.card).border_1().border_color(p.border)
                    })
                    .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.bottom_panel != panel {
                            this.toggle_bottom_panel(panel, window, cx);
                        }
                    }))
                    .child(
                        Icon::new(if panel == BottomPanel::Terminal {
                            IconName::Terminal
                        } else {
                            IconName::SquareTerminal
                        })
                        .size(px(11.))
                        .text_color(if active {
                            p.fg
                        } else {
                            p.muted_fg
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if active { p.fg } else { p.muted_fg })
                            .child(label),
                    ),
            );
        }
        header = header
            .child(div().flex_1())
            .child(
                div().flex().items_center().gap(px(2.)).children(
                    [
                        (crate::app::TerminalDock::Bottom, "▁", "Bottom"),
                        (crate::app::TerminalDock::Left, "◧", "Left"),
                        (crate::app::TerminalDock::Right, "◨", "Right"),
                    ]
                    .map(|(dock, glyph, label)| {
                        let active = self.terminal_dock == dock;
                        div()
                            .id(format!("terminal-dock-position:{label}"))
                            .aria_label(format!("Place terminal panel at {label}"))
                            .h(px(20.))
                            .w(px(20.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .when(active, |this| {
                                this.bg(p.card).border_1().border_color(p.border)
                            })
                            .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!("Move panel to the {label} edge"))
                                    .build(window, cx)
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_terminal_dock(dock, cx)),
                            )
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(if active { p.fg } else { p.muted_fg })
                                    .child(glyph),
                            )
                            .into_any_element()
                    }),
                ),
            )
            .child(
                div()
                    .id("bottom-panel-close")
                    .size(px(20.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.close_bottom_panel(cx)))
                    .child(Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg)),
            );
        let body = match self.bottom_panel {
            BottomPanel::Terminal => crate::terminal::render(self, window, cx),
            BottomPanel::Run => crate::run::render(self, window, cx),
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
            .child(div().flex_1().min_h(px(0.)).flex().flex_col().child(body));
        let horizontal_handle = div()
            .id("terminal-dock-resize")
            .aria_label("Drag to resize the terminal panel")
            .h(px(9.))
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
                .bg(p.bg)
                .child(horizontal_handle)
                .child(content)
                .into_any_element(),
            crate::app::TerminalDock::Left => div()
                .h_full()
                .w(px(width))
                .flex_shrink_0()
                .overflow_hidden()
                .flex()
                .bg(p.bg)
                .child(content)
                .child(vertical_handle)
                .into_any_element(),
            crate::app::TerminalDock::Right => div()
                .h_full()
                .w(px(width))
                .flex_shrink_0()
                .overflow_hidden()
                .flex()
                .bg(p.bg)
                .child(vertical_handle)
                .child(content)
                .into_any_element(),
        }
    }

    pub(super) fn main_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.section == SidebarSection::Dashboard {
            return crate::dashboard::render(self, cx);
        }
        if self.projects.is_empty() {
            return self.onboarding(cx);
        }
        let tabs = self.render_workbench_tabs(cx);
        let content = if self.git_history.is_active_for(self.project()) {
            crate::git_history::render(self, cx)
        } else if self.workbench.show_file
            && let Some(inspector) = &self.inspector
        {
            self.inspector_panel(inspector, cx)
        } else {
            match self.view {
                MainView::Chat => views::chat::render(self, window, cx),
                MainView::Graph => views::graph::render(self, window, cx),
                MainView::Trace => views::trace::render(self, window, cx),
            }
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
            .child(tabs)
            .child(content)
            .into_any_element()
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
                div()
                    .id("editor-prompt")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(400.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
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
                                    .rounded(px(8.))
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
                            .child(
                                div()
                                    .id("editor-cancel")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_pending_editor(cx);
                                    }))
                                    .child(
                                        div().text_size(px(12.0)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("editor-discard")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.red_soft))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_pending_editor(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(p.red)
                                            .child("Discard"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("editor-save")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.9))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_pending_editor(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child("Save"),
                                    ),
                            ),
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
                div()
                    .id("diff-dialog")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(880.))
                    .h(px(560.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
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
                                    .rounded(px(7.))
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
                                    .rounded(px(4.))
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
                                div()
                                    .id("diff-open-file")
                                    .h(px(26.))
                                    .px(px(9.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_diff(window, cx);
                                        this.open_file(&open_path, window, cx);
                                    }))
                                    .child(icon(IconName::ExternalLink, 10.5, p.muted_fg))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(p.fg)
                                            .child("Open file"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("diff-close")
                                    .size(px(26.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_diff(window, cx)
                                    }))
                                    .child(icon(IconName::X, 12.0, p.muted_fg)),
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
                    .rounded(px(6.))
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
                    div()
                        .id(format!("remote-project:{}", info.id))
                        .h(px(42.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_remote_project(&host, &info_for_click, cx);
                        }))
                        .child(
                            div()
                                .size(px(24.))
                                .rounded(px(6.))
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
                div()
                    .id("remote-picker")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(440.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(28.))
                                    .rounded(px(7.))
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
                                div()
                                    .id("remote-picker-close")
                                    .size(px(24.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.remote_picker = None;
                                        cx.notify();
                                    }))
                                    .child(icon(IconName::X, 11.0, p.muted_fg)),
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
                div()
                    .id("new-file-modal")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(380.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
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
                    .child(
                        div()
                            .h(px(30.))
                            .px(px(7.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.new_file_input)
                                    .appearance(false)
                                    .bordered(false),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(7.))
                            .child(
                                div()
                                    .id("new-file-cancel")
                                    .h(px(27.))
                                    .px(px(9.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.explorer.busy {
                                            return;
                                        }
                                        this.new_file_prompt = false;
                                        cx.notify();
                                    }))
                                    .child(
                                        div().text_size(px(11.5)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("new-file-create")
                                    .h(px(27.))
                                    .px(px(9.))
                                    .rounded(px(5.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.create_remote_file(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child(if self.explorer.busy {
                                                if renaming {
                                                    "Renaming…"
                                                } else {
                                                    "Creating…"
                                                }
                                            } else {
                                                if renaming { "Rename" } else { "Create" }
                                            }),
                                    ),
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
                div()
                    .id("editor-conflict")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(430.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
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
                                    .rounded(px(8.))
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
                                div()
                                    .id("conflict-cancel")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_editor_conflict(cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(p.fg)
                                            .child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("conflict-reload")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload_editor_conflict(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(p.fg)
                                            .child("Reload"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("conflict-overwrite")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .bg(p.amber)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.9))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.overwrite_editor_conflict(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child("Overwrite"),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn project_settings_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(index) = self.project_settings_target else {
            return div().into_any_element();
        };
        let Some(project) = self.projects.get(index).cloned() else {
            return div().into_any_element();
        };
        let selected_harness = project.settings.harness.clone();
        let selected_permission = project.settings.permission_mode;
        type ChipAction = Box<dyn Fn(&mut DunlinApp, &mut Context<DunlinApp>)>;

        let chip =
            |id: String, label: &str, active: bool, cx: &mut Context<Self>, action: ChipAction| {
                div()
                    .id(id)
                    .h(px(25.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(if active { p.blue } else { p.border })
                    .bg(if active { p.blue_soft } else { p.card })
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if active { p.blue } else { p.fg })
                            .child(label.to_string()),
                    )
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
            .on_click(cx.listener(|this, _, _, cx| this.close_project_settings(cx)))
            .child(
                div()
                    .id("project-settings-modal")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(460.))
                    .max_h(px(620.))
                    .overflow_y_scroll()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
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
                                div()
                                    .id("project-settings-close")
                                    .size(px(22.))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_project_settings(cx);
                                    }))
                                    .child(
                                        Icon::new(IconName::X).size(px(12.)).text_color(p.muted_fg),
                                    ),
                            ),
                    )
                    .child(div().text_size(px(11.0)).text_color(p.muted_fg).child(
                        "Overrides apply to this project. Defaults inherit from its group ancestry, then global settings.",
                    ))
                    .child(div().flex().items_center().gap(px(8.))
                        .child(crate::project_icons::render(self, &project, 32., &p))
                        .child(div().flex_1().text_size(px(12.)).child("Project icon"))
                        .child(chip("choose-project-icon".into(), "Choose image…", false, cx, Box::new(move |this, cx| this.choose_project_icon(index, cx))))
                        .child(chip("reset-project-icon".into(), "Automatic", project.settings.custom_icon.is_none(), cx, Box::new(move |this, cx| this.reset_project_icon(index, cx)))))
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
                                div()
                                    .id("project-settings-reset")
                                    .h(px(26.))
                                    .px(px(8.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.red_soft))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.reset_project_settings(index, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(p.red)
                                            .child("Reset overrides"),
                                    ),
                            )
                            .child(div().flex_1())
                            .when(index == self.selected_project, |this| {
                                this.child(
                                    div()
                                        .id("project-settings-new-session")
                                        .h(px(26.))
                                        .px(px(8.))
                                        .rounded(px(5.))
                                        .border_1()
                                        .border_color(p.border)
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(p.hover))
                                         .on_click(cx.listener(|this, _, _, cx| {
                                            if this.chat_controls_busy() {
                                                this.set_notice("Wait for the current operation before starting a conversation", cx);
                                                return;
                                            }
                                            this.close_project_settings(cx);
                                            this.new_session(cx);
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(p.fg)
                                                .child("New conversation"),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("project-settings-done")
                                    .h(px(26.))
                                    .px(px(10.))
                                    .rounded(px(5.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_project_settings(cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child("Done"),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn context_menu_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let Some(state) = self.context_menu else {
            return div().into_any_element();
        };
        let viewport = window.viewport_size();
        let width = 228.0;
        let x = f32::from(state.position.x)
            .min(f32::from(viewport.width) - width - 8.0)
            .max(8.0);
        let y = f32::from(state.position.y)
            .min(f32::from(viewport.height) - 260.0)
            .max(8.0);

        let item = |id: &'static str, label: &str, color: Hsla| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(color)
                        .child(label.to_string()),
                )
        };

        let menu =
            match state.target {
                ContextTarget::Project(index) => {
                    let Some(project) = self.projects.get(index) else {
                        return div().into_any_element();
                    };
                    let missing = !project.is_remote() && !Path::new(&project.path).is_dir();
                    let pinned = project.pinned;
                    let path = project.path.clone();
                    let name = project.name.clone();
                    let mut menu = div()
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
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.section = SidebarSection::Projects;
                                this.project_group_editor = Some(index);
                                this.persist_settings(cx);
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
                    menu = menu.child(item("ctx-project-path", "Copy path", p.fg).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                            this.set_notice("Copied the project path", cx);
                        }),
                    ));
                    menu.child(div().h(px(1.)).my(px(2.)).bg(p.border)).child(
                        item("ctx-project-remove", "Remove project…", p.red).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.context_menu = None;
                                this.request_delete_project(index, cx);
                            },
                        )),
                    )
                }
                ContextTarget::Session(index) => {
                    let Some(session) = self.sessions.get(index) else {
                        return div().into_any_element();
                    };
                    let title = session.title.clone();
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
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.select_session(index, cx);
                            }),
                        ))
                        .child(
                            item("ctx-session-new", "New session", p.fg).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.context_menu = None;
                                    this.new_session(cx);
                                },
                            )),
                        )
                        .child(div().h(px(1.)).my(px(2.)).bg(p.border))
                        .child(
                            item("ctx-session-delete", "Delete session…", p.red).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.context_menu = None;
                                    this.request_delete_session(index, cx);
                                }),
                            ),
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
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 9.))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(width))
                    .p(px(5.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(menu),
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
            .rounded(px(10.))
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
                    .rounded(px(6.))
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
                div()
                    .id("toast-action")
                    .h(px(24.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .bg(p.fg)
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(move |style| style.opacity(0.9))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_toast(cx)))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.bg)
                            .child(action_label),
                    ),
            )
            .child(
                div()
                    .id("toast-dismiss")
                    .size(px(22.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_toast(cx)))
                    .child(Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg)),
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
                div()
                    .id("group-modal")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(380.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
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
                    .child(
                        div()
                            .h(px(30.))
                            .px(px(7.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.group_input)
                                    .appearance(false)
                                    .bordered(false),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .when_some(delete_id, |this, id| {
                                this.child(
                                    div()
                                        .id("group-delete")
                                        .h(px(27.))
                                        .px(px(9.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(p.red_soft))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_project_group(&id, cx);
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(p.red)
                                                .child("Delete group"),
                                        ),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("group-cancel")
                                    .h(px(27.))
                                    .px(px(9.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_group_prompt(cx);
                                    }))
                                    .child(
                                        div().text_size(px(11.0)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("group-save")
                                    .h(px(27.))
                                    .px(px(10.))
                                    .rounded(px(5.))
                                    .bg(p.fg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.9))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_group_prompt(window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.bg)
                                            .child(if editing.is_some() {
                                                "Save"
                                            } else {
                                                "Create group"
                                            }),
                                    ),
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
            PendingDelete::DiscardFile { path } => (
                "Discard file changes?".to_string(),
                format!(
                    "“{path}” will be restored to the last committed version. This cannot be undone."
                ),
                "Discard changes",
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
            .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx)))
            .child(
                div()
                    .id("delete-dialog")
                    .child(crate::effects::rounded_probe(&self.overlay_backdrop, 12.))
                    .w(px(392.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(p.card)
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
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
                                    .rounded(px(8.))
                                    .bg(p.red_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconName::Trash, 14.0, p.red)),
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
                                            .child(title),
                                    )
                                    .child(
                                        div()
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
                            .gap(px(7.))
                            .child(
                                div()
                                    .id("delete-cancel")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.bg)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(p.hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx)))
                                    .child(
                                        div().text_size(px(12.0)).text_color(p.fg).child("Cancel"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("delete-confirm")
                                    .h(px(28.))
                                    .px(px(11.))
                                    .rounded(px(6.))
                                    .bg(p.red)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(move |style| style.opacity(0.88))
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx)))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(white())
                                            .child(action),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn inspector_panel(&self, inspector: &Inspector, cx: &mut Context<Self>) -> AnyElement {
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
        let p = palette(cx);
        let dirty = self.editor_dirty;
        let readonly = self.editor_readonly;
        let snippet = inspector.snippet;

        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(p.border)
            .bg(p.card)
            .child(
                div()
                    .h(px(46.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .size(px(24.))
                            .rounded(px(6.))
                            .bg(p.blue_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::FileCode, 13.0, p.blue)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.fg)
                                            .truncate()
                                            .child(inspector.name.clone()),
                                    )
                                    .when(dirty, |this| {
                                        this.child(div().size(px(5.)).rounded_full().bg(p.amber))
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(inspector.directory.clone()),
                            ),
                    )
                    .child(crate::inline_blame::toggle(self, cx))
                    .when(dirty && !readonly, |header| {
                        header.child(
                            div()
                                .id("inspector-save")
                                .h(px(24.))
                                .px(px(7.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(p.border)
                                .bg(p.bg)
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .cursor_pointer()
                                .hover(move |style| style.bg(p.hover))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_editor(cx);
                                }))
                                .child(icon(IconName::Save, 11.5, p.muted_fg))
                                .child(div().text_size(px(11.5)).text_color(p.fg).child("Save"))
                                .when(self.show_shortcut_hints, |this| {
                                    this.child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_family(dunlin_ui::theme::code_font())
                                            .text_color(p.muted_fg.opacity(0.7))
                                            .child("⌘S"),
                                    )
                                }),
                        )
                    })
                    .child(
                        div()
                            .id("inspector-close")
                            .aria_label("Close editor")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.request_close_inspector(cx);
                            }))
                            .child(icon(IconName::X, 13.0, p.muted_fg)),
                    ),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap_4()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .border_b_2()
                            .border_color(p.fg)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.fg)
                            .child("Editor"),
                    )
                    .child(
                        div()
                            .h(px(16.))
                            .px(px(5.))
                            .rounded(px(3.))
                            .bg(p.muted)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .child(inspector.language.clone()),
                            ),
                    )
                    .when_some(snippet, |this, (line, _)| {
                        this.child(
                            div()
                                .h(px(16.))
                                .px(px(5.))
                                .rounded(px(3.))
                                .bg(p.blue_soft)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_family(dunlin_ui::theme::code_font())
                                        .text_color(p.blue)
                                        .child(format!("line {line}")),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child(if dirty {
                                "unsaved changes".to_string()
                            } else if readonly {
                                "read-only".to_string()
                            } else {
                                format_bytes(inspector.bytes)
                            }),
                    ),
            )
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
                            .child(
                                Editor::new(&self.editor)
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
                        body.child(crate::minimap::render(self, cx))
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
        let chat = self.view == MainView::Chat;
        div()
            .h(px(crate::branch_picker::STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .overflow_hidden()
            .px(px(crate::branch_picker::STATUS_BAR_INSET))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_t_1()
            .border_color(p.border)
            .bg(p.statusbar)
            .text_size(px(11.5))
            .text_color(p.muted_fg)
            .child(crate::branch_picker::status_badge(
                self,
                &self.project().id,
                &branch,
                cx,
            ))
            .when(!chat, |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(icon(IconName::FileDiff, 10.5, p.muted_fg))
                        .child(format!(
                            "{change_count} {}",
                            if change_count == 1 {
                                "change"
                            } else {
                                "changes"
                            }
                        )),
                )
                .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            })
            .child({
                let (harness_color, _) = crate::views::chat::harness_tone(&p, &self.harness.status);
                div()
                    .id("statusbar-harness")
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(5.))
                    .child(dot(harness_color, 5.0))
                    .child(match self.harness.status {
                        harness::HarnessStatus::Ready { .. } => "Ready",
                        harness::HarnessStatus::Starting => "Connecting…",
                        harness::HarnessStatus::Failed(_) => "Connection failed",
                        harness::HarnessStatus::Exited(_) => "Disconnected",
                        harness::HarnessStatus::Offline => "Offline",
                    })
                    // Notices may contain multiline provider errors. Keep them
                    // available on demand, never painted into the fixed footer.
                    .when_some(
                        self.notice.clone().or_else(|| self.harness.status.detail()),
                        |view, notice| {
                            view.tooltip(move |window, cx| {
                                Tooltip::new(notice.clone())
                                    .max_w(px(480.))
                                    .build(window, cx)
                            })
                        },
                    )
            })
            .child(div().flex_1())
            .child({
                let panel = self.bottom_panel;
                let active = panel == crate::app::BottomPanel::Terminal;
                div()
                    .id("statusbar-terminal")
                    .aria_label("Toggle terminal panel")
                    .h(px(20.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(p.card).border_1().border_color(p.border)
                    })
                    .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_bottom_panel(crate::app::BottomPanel::Terminal, window, cx)
                    }))
                    .child(icon(
                        IconName::Terminal,
                        10.5,
                        if active { p.fg } else { p.muted_fg },
                    ))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if active { p.fg } else { p.muted_fg })
                            .child("Terminal"),
                    )
            })
            .child({
                let active = self.bottom_panel == crate::app::BottomPanel::Run;
                div()
                    .id("statusbar-run")
                    .aria_label("Toggle Run panel")
                    .h(px(20.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(p.card).border_1().border_color(p.border)
                    })
                    .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_bottom_panel(crate::app::BottomPanel::Run, window, cx)
                    }))
                    .child(icon(
                        IconName::SquareTerminal,
                        10.5,
                        if active { p.fg } else { p.muted_fg },
                    ))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if active { p.fg } else { p.muted_fg })
                            .child("Run"),
                    )
            })
            .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            .child(
                div()
                    .font_family(dunlin_ui::theme::code_font())
                    .child(format!("${:.2}", session.cost_usd)),
            )
            .child(div().w(px(1.)).h(px(12.)).bg(p.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .child(dot(p.violet, 5.0))
                    .child(context_status_label(self.harness.usage, session.tokens)),
            )
            .into_any_element()
    }

    pub(crate) fn has_modal_overlay(&self) -> bool {
        self.settings_open
            || self.command_open
            || self.pending_delete.is_some()
            || self.diff_view.is_some()
            || self.remote_picker.is_some()
            || self.new_file_prompt
            || self.session_menu_open
            || self.pending_editor.is_some()
            || self.editor_conflict.is_some()
            || self.group_prompt.is_some()
            || self.context_menu.is_some()
            || self.project_settings_target.is_some()
            || self.group_settings_draft.is_some()
            || self.provider_auth.is_some()
            || self.branch_ui.is_open()
    }
    pub(crate) fn overlay_focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some(draft) = &self.group_settings_draft {
            return draft.focus_handle(cx);
        }
        if self.project_settings_target.is_some()
            && let Some(draft) = &self.project_runtime_draft
        {
            return draft.focus_handle(cx);
        }
        if let Some(focus) = self.branch_ui.input_focus(cx) {
            return focus;
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
        let settings = self
            .settings_open
            .then(|| views::settings::render(self, window, cx));
        let palette = self
            .command_open
            .then(|| views::palette::render(self, window, cx));
        let confirm = self.pending_delete.as_ref().map(|_| self.delete_modal(cx));
        let diff = self.diff_view.as_ref().map(|_| self.diff_modal(cx));
        let remote = self
            .remote_picker
            .as_ref()
            .map(|_| self.remote_picker_modal(cx));
        let new_file = self.new_file_prompt.then(|| self.new_file_modal(cx));
        let session = self.session_menu_open.then(|| self.session_menu(cx));
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
            .map(|_| self.project_settings_modal(cx));
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
            .then(|| crate::branch_picker::render(self, cx));
        div()
            .absolute()
            .inset_0()
            .size_full()
            .children(session)
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
                root.on_action(
                    cx.listener(|this, _: &crate::native_menu::NewSession, _, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.new_session(cx);
                        }
                    }),
                )
                .on_action(cx.listener(|this, _: &crate::native_menu::SaveAll, _, cx| {
                    this.save_all_tabs(cx);
                }))
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::ProjectSettings, _, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.open_project_settings(this.selected_project, cx);
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &crate::native_menu::SwitchBranch, window, cx| {
                        if this.dismiss_history_for_menu(cx) {
                            this.open_branch_picker(&this.project().id.clone(), window, cx);
                        }
                    }),
                )
                .on_action(cx.listener(
                    |this, _: &crate::native_menu::RefreshGit, _, cx| this.refresh_git_status(cx),
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
            .child(self.overlay_backdrop.begin())
            .child(content)
            .child(self.overlay_backdrop.finish())
            .into_any_element()
    }
}

fn onboarding_permission_card(cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let status = crate::platform::notification_permission();
    let (color, action): (Hsla, AnyElement) = match status.state {
        crate::platform::PermissionState::Granted => (
            p.green,
            div()
                .text_size(px(10.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.green)
                .child("On")
                .into_any_element(),
        ),
        crate::platform::PermissionState::NotDetermined => (
            p.amber,
            onboarding_secondary_button("onboarding-notifications-allow", "Enable", p)
                .on_click(cx.listener(|this, _, _, cx| this.request_notifications(cx)))
                .into_any_element(),
        ),
        _ => (
            p.amber,
            onboarding_secondary_button("onboarding-notifications-settings", "Open Settings", p)
                .on_click(cx.listener(|this, _, _, cx| {
                    match crate::platform::open_permission_settings(
                        crate::platform::PermissionKind::Notifications,
                    ) {
                        Ok(()) => this.set_notice("Opened notification settings", cx),
                        Err(error) => this.set_notice(error, cx),
                    }
                }))
                .into_any_element(),
        ),
    };
    div()
        .w(px(460.))
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Icon::new(IconName::Bell).size(px(14.)).text_color(color))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child("Notifications"),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child(status.state.label()),
                )
                .child(action),
        )
        .child(
            div()
                .text_size(px(10.5))
                .line_height(relative(1.4))
                .text_color(p.muted_fg)
                .child(status.detail),
        )
        .child(
            div()
                .text_size(px(10.0))
                .line_height(relative(1.4))
                .text_color(p.muted_fg.opacity(0.8))
                .child("Optional. Accessibility, Screen Recording, Automation and Full Disk Access can be reviewed later in Settings › Permissions."),
        )
        .into_any_element()
}

fn onboarding_primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(34.))
        .px(px(14.))
        .rounded(px(7.))
        .bg(p.fg)
        .text_color(p.bg)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.opacity(0.9))
        .child(label.into())
}

fn onboarding_secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(9.))
        .rounded(px(5.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .text_color(p.fg)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .child(label.into())
}

fn onboarding_text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(32.))
        .px(px(8.))
        .rounded(px(6.))
        .text_color(p.muted_fg)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.text_color(p.fg).bg(p.hover))
        .child(label.into())
}
