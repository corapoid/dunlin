//! Explicit local operator action. Never exposed through HTTP, MCP or a runner.
use crate::store::{Store, mcp_oauth::LegacyReview};
use dunlin_mcp_client::oauth::persistence::PersistenceError;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
const USAGE: &str = "mcp-oauth-legacy {show|recover} --db ABSOLUTE_EXISTING_DB --workspace-id ID --connection-id ID --actor-user-id ID";

/// This capability records a human operator declaration, not a machine proof.
/// Its fields are private and it is never deserializable from a remote request.
pub(crate) struct StoppedLegacyRelays {
    actor: String,
    change: String,
    uid: u32,
}
impl StoppedLegacyRelays {
    pub(crate) fn actor(&self) -> &str {
        &self.actor
    }
    pub(crate) fn change_id(&self) -> &str {
        &self.change
    }
    pub(crate) fn uid(&self) -> u32 {
        self.uid
    }
}
enum Action {
    Show,
    Recover {
        expected: u64,
        sha: String,
        change: String,
    },
}
struct Command {
    db: PathBuf,
    workspace: String,
    connection: String,
    actor: String,
    action: Action,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn parse(args: &[String]) -> Result<Command, String> {
    if args.len() > 24 {
        return Err(USAGE.into());
    }
    let action = args.first().map(String::as_str).ok_or(USAGE)?;
    let allowed: &[&str] = match action {
        "show" => &[],
        "recover" => &[
            "--expected-revision",
            "--review-sha256",
            "--change-id",
            "--ack-all-legacy-relays-stopped",
        ],
        _ => return Err(USAGE.into()),
    };
    let mut flags = BTreeMap::new();
    let mut index = 1;
    while index < args.len() {
        let name = args[index].as_str();
        if ![
            "--db",
            "--workspace-id",
            "--connection-id",
            "--actor-user-id",
        ]
        .contains(&name)
            && !allowed.contains(&name)
        {
            return Err("Unknown maintenance flag or extra argument".into());
        }
        let value = if name == "--ack-all-legacy-relays-stopped" {
            index += 1;
            "acknowledged"
        } else {
            let v = args
                .get(index + 1)
                .filter(|s| !s.starts_with("--") && s.len() <= 4096)
                .ok_or("Missing bounded maintenance flag value")?;
            index += 2;
            v.as_str()
        };
        if flags.insert(name, value).is_some() {
            return Err("Repeated maintenance flag".into());
        }
    }
    let get = |key| {
        flags
            .get(key)
            .copied()
            .ok_or_else(|| format!("Missing {key}"))
    };
    let db = get("--db")?;
    if !Path::new(db).is_absolute() || db.chars().any(char::is_control) {
        return Err("Explicit absolute existing database required".into());
    }
    let workspace = get("--workspace-id")?;
    let connection = get("--connection-id")?;
    let actor = get("--actor-user-id")?;
    if ![workspace, connection, actor].into_iter().all(identifier) {
        return Err("Invalid explicit maintenance identity".into());
    }
    let action = if action == "show" {
        Action::Show
    } else {
        get("--ack-all-legacy-relays-stopped")?;
        let revision = get("--expected-revision")?;
        if revision.is_empty() || !revision.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Invalid expected revision".into());
        }
        let expected = revision
            .parse::<u64>()
            .ok()
            .filter(|n| (1..9_007_199_254_740_991).contains(n))
            .ok_or("Invalid expected revision")?;
        let sha = get("--review-sha256")?;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Invalid review digest".into());
        }
        let change = get("--change-id")?;
        if change.is_empty()
            || change.len() > 64
            || !change
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(
                "A bounded change ID is required, not freeform notes or credentials".into(),
            );
        }
        Action::Recover {
            expected,
            sha: sha.into(),
            change: change.into(),
        }
    };
    Ok(Command {
        db: db.into(),
        workspace: workspace.into(),
        connection: connection.into(),
        actor: actor.into(),
        action,
    })
}
fn failure(e: PersistenceError) -> String {
    match e {
    PersistenceError::Unauthorized=>"Local recovery requires an active workspace owner/admin identity".into(),
    PersistenceError::Conflict=>"Legacy record, authority or worker gate changed; reread the review before a new decision".into(),
    _=>"Legacy recovery database or file unavailable".into(),
}
}
#[cfg(unix)]
fn existing(path: &Path) -> Result<PathBuf, String> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| "Existing regular owned database required")?;
    let meta = file
        .metadata()
        .map_err(|_| "Existing regular owned database required")?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
    {
        return Err(
            "Existing regular owned database without shared write permissions required".into(),
        );
    }
    let mut header = [0u8; 16];
    file.read_exact(&mut header)
        .map_err(|_| "Existing SQLite database required")?;
    if &header != b"SQLite format 3\0" {
        return Err("Existing SQLite database required".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Database path unavailable")?;
    let entry = std::fs::metadata(&canonical).map_err(|_| "Database path unavailable")?;
    if entry.dev() != meta.dev() || entry.ino() != meta.ino() {
        return Err("Database path changed".into());
    }
    Ok(canonical)
}
#[cfg(not(unix))]
fn existing(_: &Path) -> Result<PathBuf, String> {
    Err("Legacy operator recovery requires a Unix host".into())
}
fn run(args: &[String]) -> Result<String, String> {
    let command = parse(args)?;
    let path = existing(&command.db)?;
    let store = Store::open_mcp_legacy(&path, matches!(command.action, Action::Recover { .. }))
        .map_err(failure)?;
    let value: Value = match command.action {
        Action::Show => {
            let review: LegacyReview = store
                .mcp_oauth_review_legacy(&command.actor, &command.workspace, &command.connection)
                .map_err(failure)?;
            json!({"review":review,"local_stop_proof":"operator_confirmation_required","provider_outcome_changed":false,"credential_write_outcomes_changed":false})
        }
        Action::Recover {
            expected,
            sha,
            change,
        } => {
            #[cfg(unix)]
            let uid = unsafe { libc::geteuid() };
            #[cfg(not(unix))]
            let uid = 0;
            let stopped = StoppedLegacyRelays {
                actor: command.actor,
                change,
                uid,
            };
            let result = store
                .mcp_oauth_recover_legacy(
                    &command.workspace,
                    &command.connection,
                    expected,
                    &sha,
                    &stopped,
                )
                .map_err(failure)?;
            json!({"connection_id":command.connection,"revision":result.connection_revision,"confirmed":result.confirmed,"unconfirmed":result.unconfirmed,"provider_revocation":if result.confirmed_known_tokens {"confirmed_known_tokens"}else{"unconfirmed"},"local_stop_proof":"operator_attestation","new_confirmations":0,"credential_write_outcomes_changed":false})
        }
    };
    serde_json::to_string_pretty(&value).map_err(|_| "Maintenance summary unavailable".into())
}
pub(crate) fn command(args: &[String]) -> Result<(), String> {
    if args == ["--help"] {
        println!(
            "{USAGE}\nrecover additionally requires --expected-revision N --review-sha256 SHA256 --change-id CHANGE_ID --ack-all-legacy-relays-stopped\nStop every legacy relay process and disable its automatic restart before recovery. The flag is an operator declaration, not a verified process or provider outcome. No HTTP or secret-store I/O is performed."
        );
        return Ok(());
    }
    println!("{}", run(args)?);
    Ok(())
}
#[cfg(all(test, unix))]
mod tests;
