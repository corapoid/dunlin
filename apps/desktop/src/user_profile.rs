//! Account menu backed by authenticated gh/glab profiles; no token access.
use crate::app::{DunlinApp, UiDensity};
use dunlin_http as ureq;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{Disableable, Icon, Selectable, popover::Popover},
    *,
};
use dunlin_workspace::{
    git::hosting::Provider,
    hosting_account::{self, Profile},
};
use std::{
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct State {
    pub profiles: Vec<(Profile, Option<Arc<Image>>)>,
    pub checked: Option<Instant>,
    pub menu_open: bool,
    pub github_accounts: Vec<hosting_account::GitHubAccount>,
    switching: bool,
    busy: bool,
}
fn avatar_url(value: &str) -> Option<url::Url> {
    if value.len() > 2048 {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some("avatars.githubusercontent.com" | "gitlab.com" | "secure.gravatar.com")
        ))
    .then_some(url)
}
fn avatar(value: &str) -> Option<Arc<Image>> {
    let url = avatar_url(value)?;
    let response = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(8))
        .build()
        .get(url.as_str())
        .set("Accept", "image/png, image/jpeg")
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    crate::project_icons::decode(&bytes, "png").ok()
}
impl DunlinApp {
    pub(crate) fn refresh_user_profile(&mut self, cx: &mut Context<Self>) {
        if self.user_profile.busy
            || self
                .user_profile
                .checked
                .is_some_and(|time| time.elapsed() < Duration::from_secs(300))
        {
            return;
        }
        self.user_profile.busy = true;
        cx.spawn(async |this, cx| {
            let (profiles, accounts) = cx
                .background_executor()
                .spawn(async {
                    let accounts = hosting_account::github_accounts().unwrap_or_default();
                    let profiles = [Provider::GitHub, Provider::GitLab]
                        .into_iter()
                        .filter_map(|provider| {
                            let profile = hosting_account::load(provider).ok()?;
                            let image = profile.avatar_url.as_deref().and_then(avatar);
                            Some((profile, image))
                        })
                        .collect();
                    (profiles, accounts)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.user_profile.profiles = profiles;
                this.user_profile.github_accounts = accounts;
                this.user_profile.checked = Some(Instant::now());
                this.user_profile.busy = false;
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn switch_github_account(&mut self, login: String, cx: &mut Context<Self>) {
        if self.user_profile.switching
            || self.user_profile.busy
            || !self
                .user_profile
                .github_accounts
                .iter()
                .any(|account| account.login == login && !account.active)
        {
            return;
        }
        self.user_profile.switching = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { hosting_account::switch_github_account(&login) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.user_profile.switching = false;
                match result {
                    Ok(()) => {
                        this.user_profile.checked = None;
                        this.refresh_user_profile(cx);
                        this.set_notice("Active GitHub account switched", cx);
                    }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn set_ui_density(&mut self, density: UiDensity, cx: &mut Context<Self>) {
        self.density = density;
        dunlin_ui::scale::set_density(if density == UiDensity::Comfortable {
            dunlin_ui::scale::Density::Comfortable
        } else {
            dunlin_ui::scale::Density::Compact
        });
        self.persist_settings(cx);
        cx.notify();
    }
}
pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let preferred = this.git.repository.as_ref().map(|link| &link.provider);
    let account = this
        .user_profile
        .profiles
        .iter()
        .find(|(profile, _)| Some(&profile.provider) == preferred)
        .or_else(|| this.user_profile.profiles.first());
    let label = account
        .map(|(profile, _)| {
            format!(
                "{} · @{} · {}",
                profile.name,
                profile.login,
                if profile.provider == Provider::GitHub {
                    "GitHub"
                } else {
                    "GitLab"
                }
            )
        })
        .unwrap_or_else(|| "User menu · no hosting account connected".into());
    let image = account.and_then(|(_, image)| image.clone());
    let name = account
        .map(|(profile, _)| profile.name.clone())
        .unwrap_or_else(|| "Dunlin".into());
    let identity = account
        .map(|(profile, _)| {
            format!(
                "@{} · {}",
                profile.login,
                if profile.provider == Provider::GitHub {
                    "GitHub"
                } else {
                    "GitLab"
                }
            )
        })
        .unwrap_or_else(|| "No account connected".into());
    let initial = account
        .and_then(|(profile, _)| profile.name.chars().next())
        .map(|c| c.to_uppercase().to_string());
    let face = div()
        .size(px(22.))
        .text_size(px(11.))
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("user-avatar")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .rounded_full()
        .bg(p.accent)
        .flex()
        .items_center()
        .justify_center()
        .map(|face| {
            if let Some(image) = image {
                face.child(img(image).size(px(22.)).rounded_full())
            } else if let Some(initial) = initial {
                face.child(initial)
            } else {
                face.child(Icon::new(assets::IconName::User).size(px(14.)))
            }
        });
    let trigger = dunlin_ui::button("title-user-menu")
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("title-user-menu")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .size(px(32.))
        .rounded_full()
        .p(px(0.))
        .accessibility_label(label.clone())
        .tooltip(label.clone())
        .child(face);
    let app = cx.entity().downgrade();
    let content_app = app.clone();
    Popover::new("profile-popover")
        .anchor(Anchor::TopRight)
        .offset(px(6.))
        .trigger(trigger)
        .on_open_change(move |open, _, cx| {
            let app = app.clone();
            let open = *open;
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    this.user_profile.menu_open = open;
                    if open {
                        this.refresh_user_profile(cx);
                    }
                    cx.notify();
                });
            });
        })
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            content_app
                .update(cx, |this, cx| {
                    let p = palette(cx);
                    div()
                        .id("profile-menu")
                        .w(px(200.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .role(Role::Dialog)
                        .aria_label("User menu")
                        .relative()
                        .children({
                            #[cfg(debug_assertions)]
                            {
                                crate::workspace_ui_smoke::probe("profile-menu")
                            }
                            #[cfg(not(debug_assertions))]
                            {
                                None::<AnyElement>
                            }
                        })
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(4.))
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(name.clone()),
                                )
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(px(11.5))
                                        .text_color(p.muted_fg)
                                        .child(identity.clone()),
                                ),
                        )
                        .child(div().h(px(1.)).bg(p.border))
                        // A real control group, not an interactive menu item: there
                        // is no row-wide hover/selection paint behind the segments.
                        .child(crate::views::settings::profile_appearance_picker(this, cx))
                        .child(
                            div()
                                .px(px(6.))
                                .h(px(30.))
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(div().flex_1().text_size(px(12.)).child("Minimal view"))
                                .child(
                                    div()
                                        .relative()
                                        .child(
                                            dunlin_ui::switch(
                                                "profile-minimal-view",
                                                "Minimal view",
                                                this.density == UiDensity::Compact,
                                            )
                                            .on_click(
                                                cx.listener(|this, checked: &bool, _, cx| {
                                                    this.set_ui_density(
                                                        if *checked {
                                                            UiDensity::Compact
                                                        } else {
                                                            UiDensity::Comfortable
                                                        },
                                                        cx,
                                                    );
                                                }),
                                            ),
                                        )
                                        .children({
                                            #[cfg(debug_assertions)]
                                            {
                                                crate::workspace_ui_smoke::probe("profile-density")
                                            }
                                            #[cfg(not(debug_assertions))]
                                            {
                                                None::<AnyElement>
                                            }
                                        }),
                                ),
                        )
                        .when(this.user_profile.github_accounts.len() > 1, |menu| {
                            menu.child(div().h(px(1.)).bg(p.border))
                                .child(
                                    div()
                                        .px(px(6.))
                                        .text_size(px(10.))
                                        .text_color(p.muted_fg)
                                        .child("GitHub accounts"),
                                )
                                .children(this.user_profile.github_accounts.iter().map(|account| {
                                    let login = account.login.clone();
                                    let popover = popover.clone();
                                    menu_button(
                                        format!("profile-github:{}", account.login),
                                        format!("@{}", account.login),
                                    )
                                    .icon(if account.active {
                                        assets::IconName::Check
                                    } else {
                                        assets::IconName::Github
                                    })
                                    .selected(account.active)
                                    .toggled(account.active)
                                    .disabled(
                                        account.active
                                            || this.user_profile.switching
                                            || this.user_profile.busy,
                                    )
                                    .accessibility_label(format!(
                                        "Switch GitHub account to @{}",
                                        account.login
                                    ))
                                    .relative()
                                    .children({
                                        #[cfg(debug_assertions)]
                                        {
                                            crate::workspace_ui_smoke::probe(if account.active {
                                                "profile-github-active"
                                            } else {
                                                "profile-github-switch"
                                            })
                                        }
                                        #[cfg(not(debug_assertions))]
                                        {
                                            None::<AnyElement>
                                        }
                                    })
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.switch_github_account(login.clone(), cx);
                                            let _ = popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        },
                                    ))
                                }))
                        })
                        .into_any_element()
                })
                .unwrap_or_else(|_| div().into_any_element())
        })
        .into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_avatars_cannot_fetch_private_hosts_credentials_or_redirect_targets() {
        assert!(avatar_url("https://avatars.githubusercontent.com/u/42?v=4").is_some());
        assert!(avatar_url("https://gitlab.com/uploads/user/avatar/42/a.png").is_some());
        for value in [
            "http://gitlab.com/a.png",
            "https://localhost/a",
            "https://127.0.0.1/a",
            "https://gitlab.com.evil/a",
            "https://token@gitlab.com/a",
            "file:///a",
            "https://gitlab.com:8443/a",
        ] {
            assert!(avatar_url(value).is_none());
        }
    }
}
