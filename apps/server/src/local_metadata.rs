//! Explicit operator-local metadata approval/grants. No network, inferred roots,
//! implicit principal grants, default database, or execution admission.
use crate::store::{Store, StoreError, local_metadata::MetadataPermission};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
const MAX_REVISION: u64 = 9_007_199_254_740_991;
const USAGE: &str = "local-metadata {show|list|approve|rebind|revoke|grant|ungrant|check} --db ABSOLUTE_EXISTING_DB --project-id ID [explicit operation flags]";
#[derive(Debug)]
enum Action {
    Show,
    Approve {
        git: PathBuf,
        common: PathBuf,
        expected: Option<u64>,
    },
    Revoke {
        expected: u64,
    },
    Grant {
        user: String,
        expected: Option<u64>,
        approval: u64,
        native: u64,
        permissions: Vec<MetadataPermission>,
        ack: bool,
    },
    Ungrant {
        user: String,
        expected: u64,
    },
    Check {
        user: String,
        permission: MetadataPermission,
    },
}
#[derive(Debug)]
struct Command {
    db: PathBuf,
    project: String,
    action: Action,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn revision(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid explicit revision".into());
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|v| (1..=MAX_REVISION).contains(v))
        .ok_or_else(|| "Invalid explicit revision".into())
}
fn required<'a>(flags: &BTreeMap<&str, &'a str>, key: &str) -> Result<&'a str, String> {
    flags
        .get(key)
        .copied()
        .ok_or_else(|| format!("Missing {key}"))
}
fn expected(flags: &BTreeMap<&str, &str>) -> Result<Option<u64>, String> {
    let value = required(flags, "--expected-revision")?;
    if value == "none" {
        Ok(None)
    } else {
        revision(value).map(Some)
    }
}
fn positive_expected(flags: &BTreeMap<&str, &str>) -> Result<u64, String> {
    expected(flags)?.ok_or_else(|| "This operation requires an existing positive revision".into())
}
fn absolute(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("An explicit bounded absolute operator-local path is required".into());
    }
    Ok(path)
}
fn permission(value: &str) -> Result<MetadataPermission, String> {
    match value {
        "read" => Ok(MetadataPermission::Read),
        "write" => Ok(MetadataPermission::Write),
        _ => Err("Permission must be read or write".into()),
    }
}
fn parse(args: &[String]) -> Result<Command, String> {
    let action = args.first().map(String::as_str).ok_or(USAGE)?;
    let allowed: &[&str] = match action {
        "show" | "list" => &[],
        "approve" | "rebind" => &["--git-dir", "--common-dir", "--expected-revision"],
        "revoke" => &["--expected-revision"],
        "grant" => &[
            "--user-id",
            "--expected-revision",
            "--expected-approval-revision",
            "--expected-native-grant-revision",
            "--permissions",
            "--ack-repository-wide-metadata-write",
        ],
        "ungrant" => &["--user-id", "--expected-revision"],
        "check" => &["--user-id", "--permission"],
        _ => return Err(USAGE.into()),
    };
    let mut flags = BTreeMap::new();
    let mut index = 1;
    while index < args.len() {
        let name = args[index].as_str();
        if !["--db", "--project-id"].contains(&name) && !allowed.contains(&name) {
            return Err("Unknown local-metadata flag or extra argument".into());
        }
        let value = if name == "--ack-repository-wide-metadata-write" {
            index += 1;
            "acknowledged"
        } else {
            let value = args
                .get(index + 1)
                .ok_or("Missing local-metadata flag value")?
                .as_str();
            if value.starts_with("--") {
                return Err("Missing local-metadata flag value".into());
            }
            index += 2;
            value
        };
        if flags.insert(name, value).is_some() {
            return Err("Repeated local-metadata flag".into());
        }
    }
    let db = absolute(required(&flags, "--db")?)?;
    let project = required(&flags, "--project-id")?;
    if !identifier(project) {
        return Err("Invalid explicit project ID".into());
    }
    let user = || -> Result<String, String> {
        let value = required(&flags, "--user-id")?;
        if !identifier(value) {
            return Err("Invalid explicit user ID".into());
        }
        Ok(value.into())
    };
    let action = match action {
        "show" | "list" => Action::Show,
        "approve" | "rebind" => {
            let expected = expected(&flags)?;
            if (action == "approve") != expected.is_none() {
                return Err(
                    "approve requires none; rebind requires the current positive approval revision"
                        .into(),
                );
            }
            Action::Approve {
                git: absolute(required(&flags, "--git-dir")?)?,
                common: absolute(required(&flags, "--common-dir")?)?,
                expected,
            }
        }
        "revoke" => Action::Revoke {
            expected: positive_expected(&flags)?,
        },
        "grant" => {
            let mut permissions = required(&flags, "--permissions")?
                .split(',')
                .map(permission)
                .collect::<Result<Vec<_>, _>>()?;
            permissions.sort();
            if permissions.is_empty()
                || permissions.len() > 2
                || permissions.windows(2).any(|w| w[0] == w[1])
                || !permissions.contains(&MetadataPermission::Read)
            {
                return Err(
                    "Grant requires explicit read, or read,write permissions without duplicates"
                        .into(),
                );
            }
            let ack = flags.contains_key("--ack-repository-wide-metadata-write");
            if permissions.contains(&MetadataPermission::Write) != ack {
                return Err("Write requires --ack-repository-wide-metadata-write; readonly grants must omit it".into());
            }
            Action::Grant {
                user: user()?,
                expected: expected(&flags)?,
                approval: revision(required(&flags, "--expected-approval-revision")?)?,
                native: revision(required(&flags, "--expected-native-grant-revision")?)?,
                permissions,
                ack,
            }
        }
        "ungrant" => Action::Ungrant {
            user: user()?,
            expected: positive_expected(&flags)?,
        },
        "check" => Action::Check {
            user: user()?,
            permission: permission(required(&flags, "--permission")?)?,
        },
        _ => return Err(USAGE.into()),
    };
    Ok(Command {
        db,
        project: project.into(),
        action,
    })
}
fn failure(error: StoreError) -> String {
    match error {
        StoreError::Io(_) => "Local metadata filesystem unavailable".into(),
        StoreError::Sqlite(_) => "Local metadata database unavailable".into(),
        StoreError::Encoding(_) => "Invalid local metadata record".into(),
        StoreError::Policy(_) => "Local metadata authority denied".into(),
        StoreError::Conflict(_) => {
            "Local metadata changed; reread explicit revisions before a new decision".into()
        }
        StoreError::Limit(message) => message,
    }
}
fn execute(store: &Store, command: &Command) -> Result<Value, String> {
    let project = &command.project;
    let value=(||->crate::store::Result<Value>{match &command.action{
  Action::Show=>Ok(json!({"approval":store.local_metadata(project)?,"grants":store.local_metadata_grants(project)?,"observed_state_only":true,"execution_ready":false})),
  Action::Approve{git,common,expected}=>{let preparation=store.prepare_local_metadata(project)?;let prepared=preparation.capture(git,common)?;Ok(serde_json::to_value(store.commit_local_metadata(prepared,*expected)?)?)},
  Action::Revoke{expected}=>Ok(serde_json::to_value(store.revoke_local_metadata(project,*expected)?)?),
  Action::Grant{user,expected,approval,native,permissions,ack}=>Ok(serde_json::to_value(store.set_local_metadata_grant(project,user,*approval,*native,permissions,true,*expected,*ack)?)?),
  Action::Ungrant{user,expected}=>Ok(serde_json::to_value(store.revoke_local_metadata_grant(project,user,*expected)?)?),
  Action::Check{user,permission}=>{let candidate=store.local_metadata_candidate(project,user,*permission)?;let validated=candidate.validate_filesystem()?;let authority=store.authorize_local_metadata(validated)?;authority.validate_filesystem()?;store.revalidate_local_metadata(&authority)?;Ok(json!({"stamp":authority.summary(),"permission":permission,"observed_state_only":true,"execution_ready":false}))},
 }})().map_err(failure)?;
    Ok(value)
}
fn existing_database(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "An existing relay database is required")?;
    if !file
        .metadata()
        .map_err(|_| "Relay database unavailable")?
        .is_file()
    {
        return Err("An existing regular relay database is required".into());
    }
    let mut header = [0u8; 16];
    file.read_exact(&mut header)
        .map_err(|_| "An existing relay database is required")?;
    if &header != b"SQLite format 3\0" {
        return Err("An existing SQLite relay database is required".into());
    }
    Ok(())
}
fn run(args: &[String]) -> Result<String, String> {
    let command = parse(args)?;
    existing_database(&command.db)?;
    let store = Store::open(&command.db).map_err(failure)?;
    let value = execute(&store, &command)?;
    let output =
        serde_json::to_string_pretty(&value).map_err(|_| "Local metadata summary unavailable")?;
    if output.len() > 128 * 1024 {
        return Err("Local metadata output exceeds its budget".into());
    }
    Ok(output)
}
pub(crate) fn command(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "{USAGE}\napprove/rebind: --git-dir PATH --common-dir PATH --expected-revision none|N\ngrant: --user-id ID --expected-revision none|N --expected-approval-revision N --expected-native-grant-revision N --permissions read|read,write [--ack-repository-wide-metadata-write]\nrevoke: --expected-revision N; ungrant: --user-id ID --expected-revision N\ncheck: --user-id ID --permission read|write"
        );
        return Ok(());
    }
    println!("{}", run(args)?);
    Ok(())
}
#[cfg(test)]
mod tests;
