//! Account menu backed by authenticated gh/glab profiles; no token access.
use crate::app::{DunlinApp, SettingsTab, UiDensity};
use dunlin_http as ureq;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{
        Icon,
        menu::{DropdownMenu, PopupMenuItem},
    },
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
            let profiles = cx
                .background_executor()
                .spawn(async {
                    [Provider::GitHub, Provider::GitLab]
                        .into_iter()
                        .filter_map(|provider| {
                            let profile = hosting_account::load(provider).ok()?;
                            let image = profile.avatar_url.as_deref().and_then(avatar);
                            Some((profile, image))
                        })
                        .collect()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.user_profile.profiles = profiles;
                this.user_profile.checked = Some(Instant::now());
                this.user_profile.busy = false;
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
    dunlin_ui::button("title-user-menu")
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
        .child(face)
        .dropdown_menu_with_anchor(Anchor::BottomRight, {
            let app = cx.entity().downgrade();
            move |menu, _, cx| {
                let Some(entity) = app.upgrade() else {
                    return menu;
                };
                entity.update(cx, |this, cx| this.refresh_user_profile(cx));
                let density_row_app = app.clone();
                let theme_app = app.clone();
                let settings_app = app.clone();
                let comfortable = entity.read(cx).density == UiDensity::Comfortable;
                menu.max_w(px(280.))
                    .label(name.clone())
                    .label(identity.clone())
                    .separator()
                    .item(
                        PopupMenuItem::new("Toggle light / dark theme")
                            .icon(assets::IconName::Sun)
                            .on_click(move |_, window, cx| {
                                let _ = theme_app
                                    .update(cx, |this, cx| this.toggle_appearance(window, cx));
                            }),
                    )
                    .item(
                        PopupMenuItem::element(move |_, _| {
                            div()
                                .relative()
                                .children({
                                    #[cfg(debug_assertions)]
                                    {
                                        crate::workspace_ui_smoke::probe("profile-density")
                                    }
                                    #[cfg(not(debug_assertions))]
                                    {
                                        None::<AnyElement>
                                    }
                                })
                                .min_w(px(190.))
                                .py(px(2.))
                                .child("Comfortable spacing")
                                .into_any_element()
                        })
                        .checked(comfortable)
                        .on_click(move |_, _, cx| {
                            let _ = density_row_app.update(cx, |this, cx| {
                                this.set_ui_density(
                                    if this.density == UiDensity::Comfortable {
                                        UiDensity::Compact
                                    } else {
                                        UiDensity::Comfortable
                                    },
                                    cx,
                                );
                            });
                        }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Application settings")
                            .icon(assets::IconName::Settings)
                            .on_click(move |_, _, cx| {
                                let _ = settings_app.update(cx, |this, cx| {
                                    if !this.dismiss_history_for_menu(cx) {
                                        return;
                                    }
                                    this.settings_tab = SettingsTab::General;
                                    this.settings_open = true;
                                    cx.notify();
                                });
                            }),
                    )
            }
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
