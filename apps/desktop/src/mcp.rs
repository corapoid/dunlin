//! Session-only MCP exclusions. Never disconnect shared/location-wide servers.
use crate::{
    app::DunlinApp,
    provider_auth::local_request,
    providers::{api, encode},
};
use dunlin_ui::Context;
use serde_json::{Value, json};

const OWNED: &str = "dunlin.mcp_disabled";

#[derive(Clone)]
pub(crate) struct Server {
    pub name: String,
    pub connected: bool,
    pub disabled: bool,
    pub toggleable: bool,
    status: String,
}

impl Server {
    pub(crate) fn label(&self) -> &str {
        if self.disabled {
            "off for this session"
        } else {
            &self.status
        }
    }
}

#[derive(Default)]
pub(crate) struct Catalog {
    directory: String,
    session: String,
    pub servers: Vec<Server>,
    pub loading: bool,
    pub changing: bool,
    pub error: Option<String>,
    generation: u64,
}

impl Catalog {
    pub(crate) fn active_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|s| s.connected && !s.disabled)
            .count()
    }
}

fn normalized(name: &str) -> String {
    // OpenCode normalizes in JavaScript without the Unicode regex flag, so
    // supplementary characters become one underscore per UTF-16 code unit.
    name.encode_utf16()
        .map(|unit| {
            let c = char::from_u32(u32::from(unit)).unwrap_or('_');
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn rules(name: &str) -> Vec<Value> {
    vec![
        json!({"action":format!("{}_*", normalized(name)),"resource":"*","effect":"deny"}),
        json!({"action":"opencode_list_mcp_resources","resource":name,"effect":"deny"}),
        json!({"action":"opencode_read_mcp_resource","resource":format!("{name}:*"),"effect":"deny"}),
    ]
}

fn safe_name(name: &str, names: &[String]) -> bool {
    !name.is_empty()
        && !normalized(name).starts_with("opencode")
        && !name.contains(['*', '?', '[', ']'])
        && !names.iter().any(|other| {
            other != name && {
                let a = normalized(name);
                let b = normalized(other);
                a == b || a.starts_with(&format!("{b}_")) || b.starts_with(&format!("{a}_"))
            }
        })
}

fn session_data<'a>(response: &'a Value, id: &str) -> Result<&'a Value, String> {
    let data = &response["data"];
    if data["id"].as_str() != Some(id) {
        return Err("The OpenCode session changed. Refresh MCP and try again.".into());
    }
    Ok(data)
}

fn check_location(session: &Value, directory: &str) -> Result<(), String> {
    if session["location"]["directory"]
        .as_str()
        .is_some_and(|actual| actual != directory)
    {
        return Err("This OpenCode session belongs to another location. Resume it in the correct project before changing MCP.".into());
    }
    Ok(())
}

fn parse_servers(response: &Value, session: &Value) -> Result<Vec<Server>, String> {
    let rows = response["data"]
        .as_array()
        .ok_or("Unsupported MCP catalog")?;
    let names: Vec<_> = rows
        .iter()
        .filter_map(|s| s["name"].as_str().map(str::to_owned))
        .collect();
    let permissions = session["permissions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let owned = session["metadata"][OWNED].as_object();
    let mut servers: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let name = row["name"].as_str()?;
            let status = row["status"]["status"].as_str().unwrap_or("unavailable");
            let mine = owned.is_some_and(|owned| owned.contains_key(name));
            let denied = permissions.contains(&rules(name)[0]);
            Some(Server {
                name: name.into(),
                connected: status == "connected",
                disabled: denied,
                toggleable: safe_name(name, &names)
                    && (status == "connected" || mine)
                    && (!denied || mine),
                status: match status {
                    "connected" => "connected",
                    "needs_auth" => "sign-in needed",
                    "disabled" => "disconnected",
                    "failed" => "connection failed",
                    _ => "unavailable",
                }
                .into(),
            })
        })
        .collect();
    servers.sort_by_key(|s| s.name.to_lowercase());
    Ok(servers)
}

fn patch(session: &Value, name: &str, disable: bool) -> Result<Value, String> {
    for (value, valid) in [
        (&session["permissions"], session["permissions"].is_array()),
        (&session["metadata"], session["metadata"].is_object()),
    ] {
        if !value.is_null() && !valid {
            return Err("Unsupported session settings; no MCP permissions were changed.".into());
        }
    }
    if session["metadata"]
        .get(OWNED)
        .is_some_and(|value| !value.is_object())
    {
        return Err("Unsupported Dunlin MCP metadata; no permissions were changed.".into());
    }
    let mut permissions = session["permissions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut metadata = session["metadata"].as_object().cloned().unwrap_or_default();
    let mut owned = metadata
        .get(OWNED)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if disable {
        let mut tracked = match owned.get(name) {
            Some(value) => value
                .as_array()
                .filter(|entries| entries.iter().all(|rule| rules(name).contains(rule)))
                .ok_or("MCP exclusion metadata is unsupported; no permissions were changed.")?
                .iter()
                .filter(|rule| permissions.contains(rule))
                .cloned()
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let added: Vec<_> = rules(name)
            .into_iter()
            .filter(|rule| !permissions.contains(rule))
            .collect();
        if added.is_empty() {
            return Err(
                "MCP is disabled by an existing policy; Dunlin will not remove that policy.".into(),
            );
        }
        permissions.extend(added.clone());
        tracked.extend(added);
        owned.insert(name.into(), Value::Array(tracked));
    } else {
        let Some(added) = owned.remove(name).and_then(|v| v.as_array().cloned()) else {
            return Err("Only exclusions created by Dunlin can be removed here.".into());
        };
        for rule in added {
            if !rules(name).contains(&rule) {
                return Err(
                    "MCP exclusion metadata is unsupported; no permissions were changed.".into(),
                );
            }
            if let Some(index) = permissions.iter().rposition(|existing| existing == &rule) {
                permissions.remove(index);
            }
        }
    }
    if owned.is_empty() {
        metadata.remove(OWNED);
    } else {
        metadata.insert(OWNED.into(), Value::Object(owned));
    }
    Ok(json!({"permissions":permissions,"metadata":metadata}))
}

impl DunlinApp {
    pub(crate) fn mcp_current(&self) -> bool {
        self.local_opencode()
            && self.mcp.directory == self.project().path
            && Some(self.mcp.session.as_str()) == self.harness.session_id.as_deref()
    }

    pub(crate) fn refresh_mcp(&mut self, cx: &mut Context<Self>) {
        if !self.local_opencode() || self.mcp_busy() {
            return;
        }
        let Some(session) = self.harness.session_id.clone() else {
            return;
        };
        if self.mcp_current() && self.mcp.loading {
            return;
        }
        let directory = self.project().path.clone();
        let generation = self.mcp.generation.wrapping_add(1);
        self.mcp = Catalog {
            directory: directory.clone(),
            session: session.clone(),
            loading: true,
            generation,
            ..Default::default()
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let servers = api(&directory, "mcp")?;
                    let response = api(&directory, &format!("session/{}", encode(&session)))?;
                    let data = session_data(&response, &session)?;
                    check_location(data, &directory)?;
                    parse_servers(&servers, data)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.mcp.generation != generation {
                    return;
                }
                this.mcp.loading = false;
                match result {
                    Ok(servers) => this.mcp.servers = servers,
                    Err(error) => this.mcp.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn toggle_mcp(&mut self, name: &str, disable: bool, cx: &mut Context<Self>) {
        if !self.mcp_current()
            || self.chat_controls_busy()
            || self.mcp.loading
            || self.mcp.changing
            || !self
                .mcp
                .servers
                .iter()
                .any(|s| s.name == name && s.toggleable)
        {
            return;
        }
        let name = name.to_owned();
        let directory = self.project().path.clone();
        let session = self.mcp.session.clone();
        let generation = self.mcp.generation;
        self.mcp_operations.insert(session.clone());
        let requested_session = session.clone();
        self.mcp.changing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let servers = api(&directory, "mcp")?;
                let resource = format!("session/{}", encode(&session));
                let response = api(&directory, &resource)?;
                let data = session_data(&response, &session)?;
                check_location(data,&directory)?;
                if data["time"]["busy"].as_f64().unwrap_or(0.) > data["time"]["idle"].as_f64().unwrap_or(0.) {
                    return Err("Wait for this session's current turn to finish before changing MCP.".into());
                }
                let current = parse_servers(&servers, data)?;
                if !current.iter().any(|s| s.name == name && s.toggleable && s.disabled != disable) {
                    return Err("MCP state changed. Refresh and try again.".into());
                }
                local_request(&directory, "PATCH", &resource, Some(patch(data, &name, disable)?))?;
                Ok::<_,String>(())
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_operations.remove(&requested_session);
                if this.mcp.generation != generation {
                    if this.harness.session_id.as_deref() == Some(&requested_session) { this.refresh_mcp(cx); }
                    return;
                }
                this.mcp.changing = false;
                match result {
                    Ok(()) => { this.refresh_mcp(cx); this.set_notice("MCP selection updated for this session's next turn. Earlier context is unchanged; no session restart is needed.", cx); }
                    Err(error) => this.set_notice(error, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn mcp_busy(&self) -> bool {
        (self.mcp_current() && self.mcp.changing)
            || self
                .harness
                .session_id
                .as_ref()
                .is_some_and(|id| self.mcp_operations.contains(id))
    }

    pub(crate) fn reset_mcp_for_navigation(&mut self) {
        let generation = self.mcp.generation.wrapping_add(1);
        self.mcp = Catalog {
            generation,
            ..Default::default()
        };
    }
}

#[cfg(test)]
mod tests;
