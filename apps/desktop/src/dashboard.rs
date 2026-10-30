//! Recorded activity only: no estimates, Git-history guesses or context tokens.
use crate::{app::DunlinApp, harness::ChatItem};
use chrono::{Days, Local, NaiveDate, TimeZone, Timelike};
use dunlin_domain::{Project, SidebarSection};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};

const ACTIVITY_COLUMNS: u64 = 36;
const ACTIVITY_ROWS: u64 = 4;
const ACTIVITY_DAYS: u64 = ACTIVITY_COLUMNS * ACTIVITY_ROWS;
const DOT_SIDE: f32 = 10.;
const DOT_GAP: f32 = 7.;
fn greeting_for_hour(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Hello, night owl",
    }
}

fn activity_grid_width() -> f32 {
    ACTIVITY_COLUMNS as f32 * DOT_SIDE + (ACTIVITY_COLUMNS - 1) as f32 * DOT_GAP
}

fn intensity(value: u64, maximum: u64) -> u8 {
    if value == 0 {
        return 0;
    }
    let relative = value as f64 / maximum.max(1) as f64;
    if relative >= 0.65 {
        3
    } else if relative >= 0.2 {
        2
    } else {
        1
    }
}

fn dot_color(level: u8, p: &Palette) -> Hsla {
    if p.bg.l > 0.5 {
        return match level {
            0 => p.muted,
            1 => p.blue.opacity(0.12),
            2 => p.blue.opacity(0.25),
            _ => p.blue.opacity(0.42),
        };
    }
    match level {
        0 => p.muted,
        1 => p.blue.opacity(0.3),
        2 => p.blue.opacity(0.65),
        _ => p.blue,
    }
}

fn activity_date(today: NaiveDate, column: u64, row: u64) -> Option<NaiveDate> {
    if column >= ACTIVITY_COLUMNS || row >= ACTIVITY_ROWS {
        return None;
    }
    today.checked_sub_days(Days::new(ACTIVITY_DAYS - 1 - column * ACTIVITY_ROWS - row))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Metric {
    #[default]
    Requests,
    Tokens,
    ChangedLines,
}
impl Metric {
    pub const ALL: [Self; 3] = [Self::Requests, Self::Tokens, Self::ChangedLines];
    pub fn label(self) -> &'static str {
        match self {
            Self::Requests => "Requests",
            Self::Tokens => "Reported tokens",
            Self::ChangedLines => "Reported changed lines",
        }
    }
    fn short_label(self) -> &'static str {
        match self {
            Self::Requests => "Requests",
            Self::Tokens => "Tokens",
            Self::ChangedLines => "Changed lines",
        }
    }
    fn icon(self) -> IconName {
        match self {
            Self::Requests => IconName::MessageSquare,
            Self::Tokens => IconName::ChartNoAxesColumn,
            Self::ChangedLines => IconName::FileDiff,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProjectVisit {
    id: String,
    at_ms: u64,
}

fn remember_project(visits: &mut Vec<ProjectVisit>, id: &str, at_ms: u64) {
    visits.retain(|visit| visit.id != id);
    visits.insert(
        0,
        ProjectVisit {
            id: id.into(),
            at_ms,
        },
    );
    visits.truncate(32);
}

fn recent_projects(
    projects: &[Project],
    visits: &[ProjectVisit],
    recorded: &HashMap<String, u64>,
) -> Vec<String> {
    let mut ranked: Vec<_> = projects
        .iter()
        .filter_map(|project| {
            let at_ms = visits
                .iter()
                .filter(|visit| visit.id == project.id)
                .map(|visit| visit.at_ms)
                .chain(recorded.get(&project.id).copied())
                .max()?;
            (at_ms > 0).then_some((project.id.clone(), at_ms))
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(3).map(|(id, _)| id).collect()
}

fn last_user_activity(items: &[ChatItem]) -> Option<u64> {
    items
        .iter()
        .filter_map(|item| match item {
            ChatItem::User { at_ms, timing, .. } => Some(if *at_ms > 0 {
                *at_ms
            } else {
                timing.map(|t| t.started_at_ms).unwrap_or(0)
            }),
            _ => None,
        })
        .filter(|at| *at > 0)
        .max()
}

/// Dashboard owns the titlebar's center slot, not the conversation view tabs.
pub(crate) fn metric_switcher(
    this: &DunlinApp,
    compact: bool,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    div()
        .id("dashboard-metrics")
        .flex_shrink_0()
        .h(px(28.))
        .p(px(2.))
        .rounded(px(8.))
        .bg(p.muted)
        .flex()
        .items_center()
        .gap(px(2.))
        .children(Metric::ALL.map(|candidate| {
            let active = this.dashboard_settings.metric == candidate;
            let color = if active { p.fg } else { p.muted_fg };
            div()
                .id(format!("dashboard-metric:{candidate:?}"))
                .aria_label(format!("Show {} activity", candidate.label()))
                .h(px(24.))
                .px(px(if compact { 6. } else { 9. }))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .when(active, |tab| tab.bg(p.card))
                .when(!active, |tab| tab.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dashboard_settings.metric = candidate;
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(Icon::new(candidate.icon()).size(px(12.5)).text_color(color))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(color)
                        .child(candidate.short_label()),
                )
        }))
        .into_any_element()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub start_at_dashboard: bool,
    pub show_agents: bool,
    pub show_activity: bool,
    pub metric: Metric,
    pub recent_projects: Vec<ProjectVisit>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            start_at_dashboard: false,
            show_agents: true,
            show_activity: true,
            metric: Metric::Requests,
            recent_projects: Vec::new(),
        }
    }
}
#[derive(Default, Clone)]
pub(crate) struct Day {
    requests: u64,
    tokens: u64,
    token_samples: u64,
    missing_tokens: u64,
    lines: u64,
    change_samples: u64,
}
impl Day {
    fn value(&self, metric: Metric) -> u64 {
        match metric {
            Metric::Requests => self.requests,
            Metric::Tokens => self.tokens,
            Metric::ChangedLines => self.lines,
        }
    }
    fn available(&self, metric: Metric) -> bool {
        match metric {
            Metric::Requests => true,
            Metric::Tokens => self.token_samples > 0,
            Metric::ChangedLines => self.change_samples > 0,
        }
    }
    fn add(&mut self, other: &Self) {
        self.requests += other.requests;
        self.tokens += other.tokens;
        self.token_samples += other.token_samples;
        self.missing_tokens += other.missing_tokens;
        self.lines += other.lines;
        self.change_samples += other.change_samples;
    }
}
#[derive(Default)]
pub(crate) struct State {
    greeting: &'static str,
    days: BTreeMap<NaiveDate, Day>,
    checked: Option<Instant>,
    busy: bool,
    error: Option<String>,
    project_activity: HashMap<String, u64>,
}

fn date(at_ms: u64) -> Option<NaiveDate> {
    if at_ms == 0 {
        return None;
    }
    Local
        .timestamp_millis_opt(i64::try_from(at_ms).ok()?)
        .single()
        .map(|t| t.date_naive())
}
fn record(days: &mut BTreeMap<NaiveDate, Day>, transcript: &[ChatItem]) {
    record_inner(days, transcript, true);
}
fn record_inner(days: &mut BTreeMap<NaiveDate, Day>, transcript: &[ChatItem], root_requests: bool) {
    for (index, item) in transcript.iter().enumerate() {
        match item {
            ChatItem::User { at_ms, .. } if root_requests => {
                if let Some(date) = date(*at_ms) {
                    let day = days.entry(date).or_default();
                    day.requests += 1;
                    if let Some(tokens) = crate::projection::turn_tokens(transcript, index) {
                        day.tokens += tokens;
                        day.token_samples += 1;
                    } else {
                        day.missing_tokens += 1;
                    }
                }
            }
            ChatItem::Tool { view, at_ms, .. } => {
                if let Some(date) = date(*at_ms) {
                    // Nonzero diffs are observations, not a complete filesystem audit.
                    if view.additions > 0 || view.deletions > 0 || view.diff.is_some() {
                        let day = days.entry(date).or_default();
                        day.lines += u64::from(view.additions) + u64::from(view.deletions);
                        day.change_samples += 1;
                    }
                }
            }
            // Child usage may already be included in the root's report. Do not
            // count internal delegated prompts/tokens a second time.
            ChatItem::Child { items, .. } => record_inner(days, items, false),
            _ => {}
        }
    }
}
impl DunlinApp {
    pub(crate) fn remember_dashboard_project(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get(self.selected_project) else {
            return;
        };
        remember_project(
            &mut self.dashboard_settings.recent_projects,
            &project.id,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
        );
        self.persist_settings(cx);
    }

    pub(crate) fn refresh_dashboard(&mut self, cx: &mut Context<Self>) {
        let greeting = greeting_for_hour(Local::now().hour());
        if self.dashboard.greeting != greeting {
            self.dashboard.greeting = greeting;
            cx.notify();
        }
        if self.dashboard.busy
            || self
                .dashboard
                .checked
                .is_some_and(|at| at.elapsed().as_secs() < 5)
        {
            return;
        }
        self.dashboard.busy = true;
        let path = self.store.as_ref().and_then(|s| s.database_path());
        let ids: Vec<_> = self.all_sessions.iter().map(|s| s.id.clone()).collect();
        let session_projects: HashMap<_, _> = self
            .all_sessions
            .iter()
            .map(|s| (s.id.clone(), s.project_id.clone()))
            .collect();
        let expected_ids = ids.clone();
        let mut live = self.background_transcripts();
        live.insert(self.session().id.clone(), self.harness.transcript.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = path
                        .map(dunlin_storage::Store::read_only)
                        .transpose()
                        .map_err(|e| e.to_string())?;
                    let mut days = BTreeMap::new();
                    let mut project_activity = HashMap::<String, u64>::new();
                    for id in ids {
                        let items = match live.remove(&id) {
                            Some(items) => items,
                            None => store
                                .as_ref()
                                .map(|s| s.load_transcript::<Vec<ChatItem>>(&id))
                                .transpose()
                                .map_err(|e| e.to_string())?
                                .flatten()
                                .unwrap_or_default(),
                        };
                        if let Some(project) = session_projects.get(&id)
                            && let Some(latest) = last_user_activity(&items)
                        {
                            project_activity
                                .entry(project.clone())
                                .and_modify(|at| *at = (*at).max(latest))
                                .or_insert(latest);
                        }
                        record(&mut days, &items);
                    }
                    Ok::<_, String>((days, project_activity))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.dashboard.busy = false;
                if this
                    .all_sessions
                    .iter()
                    .map(|s| &s.id)
                    .ne(expected_ids.iter())
                {
                    this.dashboard.checked = None;
                    cx.notify();
                    return;
                }
                this.dashboard.checked = Some(Instant::now());
                match result {
                    Ok((days, project_activity)) => {
                        this.dashboard.days = days;
                        this.dashboard.project_activity = project_activity;
                        this.dashboard.error = None;
                    }
                    Err(e) => this.dashboard.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn recent_project_tiles(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let ids = recent_projects(
        &this.projects,
        &this.dashboard_settings.recent_projects,
        &this.dashboard.project_activity,
    );
    for id in &ids {
        this.check_project_remote(id, false, cx);
    }
    let mut section = div().flex().flex_col().gap(px(12.)).child(
        div()
            .font_weight(FontWeight::SEMIBOLD)
            .text_center()
            .child("Recent projects"),
    );
    if ids.is_empty() {
        return section
            .child(
                div()
                    .text_center()
                    .text_size(px(12.))
                    .text_color(p.muted_fg)
                    .child(if this.dashboard.busy && this.dashboard.checked.is_none() {
                        "Loading recent projects…"
                    } else {
                        "Your recently used projects will appear here."
                    }),
            )
            .into_any_element();
    }
    let mut tiles = div().flex().flex_wrap().justify_center().gap(px(12.));
    for id in ids {
        let Some(project) = this.projects.iter().find(|project| project.id == id) else {
            continue;
        };
        let summary = this
            .repo_summaries
            .values
            .get(&id)
            .filter(|summary| summary.path == project.path);
        let branch = summary
            .and_then(|summary| summary.branch.as_ref())
            .or(project.branch.as_ref())
            .cloned();
        let name = project.name.clone();
        let open_id = id.clone();
        tiles = tiles.child(
            div()
                .id(format!("dashboard-recent:{id}"))
                .aria_label(format!("Open recent project {name}"))
                .flex_1()
                .min_w(px(180.))
                .max_w(px(360.))
                .min_h(px(232.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.border_strong)
                .bg(p.panel)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(index) = this.projects.iter().position(|project| project.id == id) {
                        this.select_project(index, cx);
                        if this.project().id == id {
                            this.set_view(dunlin_domain::MainView::Chat, cx);
                        }
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .child(crate::project_icons::render(this, project, 26., &p))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(name),
                        ),
                )
                .children(crate::local_projects::monorepo_badge(this, project, &p))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(crate::branch_picker::badge(
                            this,
                            &project.id,
                            branch.as_deref().unwrap_or("Branches…"),
                            format!("dashboard-branch:{}", project.id),
                            cx,
                        )),
                )
                .child(crate::project_activity::render(this, project, cx))
                .child(div().flex_1())
                .child(
                    div().w_full().flex().justify_end().child(
                        div()
                            .id(format!("dashboard-open:{open_id}"))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(5.))
                            .bg(p.muted)
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .text_size(px(11.))
                            .text_color(p.fg)
                            .cursor_pointer()
                            .child("Open project")
                            .child(Icon::new(IconName::ChevronRight).size(px(12.))),
                    ),
                ),
        )
    }
    section = section.child(tiles);
    section.into_any_element()
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.refresh_dashboard(cx);
    this.refresh_project_summaries(cx);
    let p = palette(cx);
    let metric = this.dashboard_settings.metric;
    let mut footer = None;
    let mut body = div()
        .w_full()
        .max_w(px(1000.))
        .flex_shrink_0()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(20.))
        .child(
            div()
                .text_size(px(24.))
                .text_center()
                .font_weight(FontWeight::SEMIBOLD)
                .child(greeting_for_hour(Local::now().hour())),
        )
        .child(div().text_center().text_size(px(12.)).text_color(p.muted_fg).child(
            "Your projects, live agents and recorded activity · customize in Settings → General",
        ));
    let live = this.live_agents();
    if live.is_empty() {
        body = body.child(recent_project_tiles(this, cx));
    } else if this.dashboard_settings.show_agents {
        let mut agents = div().flex().flex_col().gap(px(8.)).child(
            div()
                .text_center()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Live agents"),
        );
        for (project, session, status) in live {
            let Some(index) = this.projects.iter().position(|p| p.id == project) else {
                continue;
            };
            let name = this.projects[index].name.clone();
            let title = this
                .all_sessions
                .iter()
                .find(|s| s.id == session)
                .map(|s| s.title.clone())
                .unwrap_or_default();
            agents = agents.child(
                div()
                    .id(format!("dashboard-agent:{session}"))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .p(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_project(index, cx);
                        if this.project().id == project {
                            if let Some(index) = this.sessions.iter().position(|s| s.id == session)
                            {
                                this.select_session(index, cx);
                            }
                            this.section = SidebarSection::Projects;
                            this.set_view(dunlin_domain::MainView::Chat, cx);
                        }
                    }))
                    .child(div().size(px(7.)).rounded_full().bg(p.blue))
                    .child(div().flex_1().child(format!("{name} · {title}")))
                    .child(div().text_size(px(11.)).text_color(p.blue).child(status)),
            );
        }
        body = body.child(agents);
    }
    if this.dashboard_settings.show_activity {
        let today = Local::now().date_naive();
        let mut week = Day::default();
        for offset in 0..7 {
            if let Some(day) = today
                .checked_sub_days(Days::new(offset))
                .and_then(|date| this.dashboard.days.get(&date))
            {
                week.add(day);
            }
        }
        let mut grid = div().flex().gap(px(DOT_GAP)).flex_shrink_0();
        let maximum = (0..ACTIVITY_DAYS)
            .filter_map(|n| today.checked_sub_days(Days::new(n)))
            .filter_map(|date| this.dashboard.days.get(&date))
            .map(|day| day.value(metric))
            .max()
            .unwrap_or(1)
            .max(1);
        for column in 0..ACTIVITY_COLUMNS {
            let mut days = div().flex().flex_col().gap(px(DOT_GAP)).flex_shrink_0();
            for row in 0..ACTIVITY_ROWS {
                let Some(date) = activity_date(today, column, row) else {
                    continue;
                };
                let day = this.dashboard.days.get(&date).cloned().unwrap_or_default();
                let value = day.value(metric);
                let text = if this.dashboard.busy && this.dashboard.checked.is_none() {
                    "loading".into()
                } else if day.available(metric) {
                    value.to_string()
                } else {
                    "not reported".into()
                };
                let label = format!("{date}: {text} {}", metric.label());
                let level = intensity(value, maximum);
                let tone = dot_color(level, &p);
                days = days.child(
                    div()
                        .id(format!("activity:{date}"))
                        .aria_label(label.clone())
                        .size(px(DOT_SIDE))
                        .rounded_full()
                        .bg(tone)
                        .when(level >= 2 && p.bg.l < 0.5, |dot| {
                            dot.shadow(vec![
                                BoxShadow::new(
                                    px(0.),
                                    px(0.),
                                    tone.opacity(if level == 3 { 0.55 } else { 0.18 }),
                                )
                                .blur_radius(px(if level == 3 { 9. } else { 5. }))
                                .spread_radius(px(1.)),
                            ])
                        })
                        .tooltip(move |window, cx| {
                            dunlin_ui::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        }),
                );
            }
            grid = grid.child(days);
        }
        let selected_total = if this.dashboard.checked.is_none() || !week.available(metric) {
            "—".into()
        } else {
            crate::app::format_tokens(week.value(metric))
        };
        let mut legend = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(10.))
            .text_color(p.muted_fg)
            .child("Less");
        for level in 0..4 {
            legend = legend.child(div().size(px(6.)).rounded_full().bg(dot_color(level, &p)));
        }
        legend = legend.child("More");
        body = body.child(
            div()
                .id("dashboard-activity")
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(18.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .gap(px(14.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.muted_fg)
                                        .child("ACTIVITY"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap(px(5.))
                                        .child(
                                            div()
                                                .text_size(px(30.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.fg)
                                                .child(selected_total),
                                        )
                                        .child(
                                            div().text_size(px(11.)).text_color(p.muted_fg).child(
                                                format!(
                                                    "{} · last 7 days",
                                                    metric.label().to_lowercase()
                                                ),
                                            ),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("dashboard-dot-scroll")
                        .overflow_x_scroll()
                        .min_w(px(0.))
                        .py(px(12.))
                        .px(px(8.))
                        .child(
                            div()
                                .w_full()
                                .min_w(px(activity_grid_width()))
                                .flex()
                                .justify_center()
                                .child(grid),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(p.muted_fg)
                                .child(format!(
                                    "Last {ACTIVITY_DAYS} days · one dot per day · oldest → newest"
                                )),
                        )
                        .child(legend),
                ),
        );
        footer = Some(div().id("dashboard-data-footer").w_full().flex_shrink_0().px(px(24.)).py(px(10.)).border_t_1().border_color(p.border).text_center().text_size(px(10.5)).text_color(p.muted_fg).child(format!("Recorded data only · {} requests without token reports in the last 7 days. Changed lines are tool-reported, may repeat, and are not complete Git activity. Dim dots do not prove inactivity.", week.missing_tokens)));
        if let Some(error) = &this.dashboard.error {
            body = body.child(
                div()
                    .text_center()
                    .text_color(p.red)
                    .child(format!("Activity unavailable: {error}")),
            );
        }
    }
    // Flexible spacers center short dashboards vertically without clipping the
    // top when many agents or a small window require scrolling.
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .id("dashboard")
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_y_scroll()
                .p(px(28.))
                .flex()
                .flex_col()
                .items_center()
                .child(div().flex_1().min_h(px(0.)))
                .child(body)
                .child(div().flex_1().min_h(px(0.))),
        )
        .children(footer)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn greeting_boundaries_follow_the_hour_of_the_supplied_timezone() {
        use chrono::{TimeZone, Timelike};
        for (hour, expected) in [
            (0, "Hello, night owl"),
            (4, "Hello, night owl"),
            (5, "Good morning"),
            (11, "Good morning"),
            (12, "Good afternoon"),
            (17, "Good afternoon"),
            (18, "Good evening"),
            (22, "Good evening"),
            (23, "Hello, night owl"),
        ] {
            assert_eq!(super::greeting_for_hour(hour), expected);
        }
        let utc = chrono::Utc.with_ymd_and_hms(2026, 10, 30, 4, 0, 0).unwrap();
        let east = utc.with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap());
        assert_eq!(super::greeting_for_hour(east.hour()), "Good afternoon");
        let west = utc.with_timezone(&chrono::FixedOffset::west_opt(8 * 3600).unwrap());
        assert_eq!(super::greeting_for_hour(west.hour()), "Good evening");
    }
    use super::{Day, Metric, record};
    use crate::harness::{ChatItem, TurnTiming};
    use std::collections::BTreeMap;
    #[test]
    fn activity_levels_are_monotonic_and_zero_activity_never_glows() {
        assert_eq!(super::intensity(0, 0), 0);
        assert_eq!(super::intensity(0, 100), 0);
        assert_eq!(super::intensity(1, 100), 1);
        assert_eq!(super::intensity(20, 100), 2);
        assert_eq!(super::intensity(65, 100), 3);
        for value in 0..100 {
            assert!(super::intensity(value, 100) <= super::intensity(value + 1, 100));
        }
    }

    fn project(id: &str) -> dunlin_domain::Project {
        dunlin_domain::Project {
            id: id.into(),
            name: id.into(),
            path: format!("/projects/{id}"),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    #[test]
    fn recent_tiles_merge_user_visits_with_recorded_history_and_limit_to_three_existing_projects() {
        let projects: Vec<_> = ["a", "b", "c", "d", "unused"].map(project).into();
        let recorded = std::collections::HashMap::from([
            ("a".into(), 100),
            ("b".into(), 200),
            ("c".into(), 300),
            ("d".into(), 400),
            ("deleted".into(), 9_999),
        ]);
        let visits = vec![
            super::ProjectVisit {
                id: "a".into(),
                at_ms: 500,
            },
            super::ProjectVisit {
                id: "b".into(),
                at_ms: 150,
            },
        ];
        assert_eq!(
            super::recent_projects(&projects, &visits, &recorded),
            ["a", "d", "c"]
        );
        assert!(super::recent_projects(&projects, &[], &Default::default()).is_empty());
        assert!(super::recent_projects(&[], &visits, &recorded).is_empty());
    }

    #[test]
    fn project_visits_are_unique_bounded_and_persist_without_resetting_preferences() {
        let mut settings: super::Settings = serde_json::from_str(
            r#"{"metric":"Tokens","show_agents":false,"start_at_dashboard":true}"#,
        )
        .unwrap();
        assert!(settings.recent_projects.is_empty());
        for n in 0..40 {
            super::remember_project(
                &mut settings.recent_projects,
                &format!("project-{n}"),
                n + 1,
            );
        }
        super::remember_project(&mut settings.recent_projects, "project-39", 100);
        assert_eq!(settings.recent_projects.len(), 32);
        assert_eq!(
            settings
                .recent_projects
                .iter()
                .filter(|v| v.id == "project-39")
                .count(),
            1
        );
        let restored: super::Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.metric, Metric::Tokens);
        assert!(!restored.show_agents);
        assert!(restored.start_at_dashboard);
        assert_eq!(restored.recent_projects[0].id, "project-39");
        assert_eq!(restored.recent_projects[0].at_ms, 100);
    }

    #[test]
    fn background_output_and_delayed_dispatch_do_not_reorder_recent_user_work() {
        let items = vec![
            ChatItem::User {
                text: "queued".into(),
                at_ms: 100,
                timing: Some(TurnTiming {
                    started_at_ms: 2_000,
                    finished_at_ms: None,
                    elapsed_ms: None,
                    usage: None,
                    usage_before: None,
                }),
            },
            ChatItem::AgentText {
                text: "background output".into(),
                at_ms: 3_000,
                streaming: false,
                message_id: None,
            },
        ];
        assert_eq!(super::last_user_activity(&items), Some(100));
        assert_eq!(super::last_user_activity(&items[1..]), None);
    }

    #[test]
    fn dot_colors_follow_the_existing_light_and_dark_application_palettes() {
        for p in [dunlin_ui::theme::light(), dunlin_ui::theme::dark()] {
            assert_eq!(super::dot_color(0, &p), p.muted);
            if p.bg.l > 0.5 {
                assert_eq!(super::dot_color(3, &p), p.blue.opacity(0.42));
            } else {
                assert_eq!(super::dot_color(3, &p), p.blue);
            }
        }
    }

    #[test]
    fn four_row_dot_matrix_has_exactly_one_dot_for_each_day_and_ends_today() {
        use chrono::{Days, NaiveDate};
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let mut dates = Vec::new();
        for column in 0..super::ACTIVITY_COLUMNS {
            for row in 0..super::ACTIVITY_ROWS {
                dates.push(super::activity_date(today, column, row).unwrap());
            }
        }
        assert_eq!(dates.len(), 144);
        assert_eq!(dates[0], today.checked_sub_days(Days::new(143)).unwrap());
        assert_eq!(dates.last(), Some(&today));
        assert!(
            dates
                .windows(2)
                .all(|days| days[0].checked_add_days(Days::new(1)) == Some(days[1]))
        );
        assert!(super::activity_date(today, super::ACTIVITY_COLUMNS, 0).is_none());
        assert!(super::activity_date(today, 0, super::ACTIVITY_ROWS).is_none());
    }
    #[test]
    fn unknown_tokens_and_undated_requests_are_not_invented() {
        let transcript = vec![
            ChatItem::User {
                text: "old".into(),
                timing: None,
                at_ms: 0,
            },
            ChatItem::User {
                text: "now".into(),
                timing: Some(TurnTiming {
                    started_at_ms: 1_750_000_000_000,
                    finished_at_ms: None,
                    elapsed_ms: None,
                    usage: None,
                    usage_before: None,
                }),
                at_ms: 1_750_000_000_000,
            },
        ];
        let mut days = BTreeMap::new();
        record(&mut days, &transcript);
        let mut total = Day::default();
        for day in days.values() {
            total.add(day);
        }
        assert_eq!(total.requests, 1);
        assert_eq!(total.missing_tokens, 1);
        assert!(!total.available(Metric::Tokens));
        assert_eq!(total.tokens, 0);
    }
}
