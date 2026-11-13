use super::*;
use std::collections::BTreeMap;
fn required<'a>(
    flags: &BTreeMap<&str, &'a str>,
    name: &str,
) -> std::result::Result<&'a str, String> {
    flags
        .get(name)
        .copied()
        .ok_or_else(|| format!("Missing {name}"))
}
fn expected(flags: &BTreeMap<&str, &str>) -> std::result::Result<u64, String> {
    required(flags, "--expected-revision")?
        .parse::<u64>()
        .ok()
        .filter(|v| *v <= MAX_REVISION)
        .ok_or_else(|| "Invalid expected revision".into())
}
fn principal(flags: &BTreeMap<&str, &str>) -> std::result::Result<ExportPrincipal, String> {
    let kind = match required(flags, "--principal-kind")? {
        "human" => PrincipalKind::Human,
        "service" => PrincipalKind::Service,
        _ => return Err("Principal kind must be human or service".into()),
    };
    let id = required(flags, "--principal-id")?;
    if !identifier(id) {
        return Err("Invalid remote principal ID".into());
    }
    Ok(ExportPrincipal {
        kind,
        id: id.into(),
    })
}
fn permission(value: &str) -> std::result::Result<ExportPermission, String> {
    match value {
        "read" => Ok(ExportPermission::Read),
        "execute" => Ok(ExportPermission::Execute),
        _ => Err("Export permission must be read or execute".into()),
    }
}
/// Operator CLI only: no network request, credential input or controller path.
pub(crate) async fn command(args: &[String]) -> std::result::Result<(), String> {
    const USAGE: &str = "worker export {list|register|rebind|revoke|grants|grant|ungrant|check} --config FILE --state-dir DIR [explicit operation flags]";
    let Some(action) = args.first().map(String::as_str) else {
        return Err(USAGE.into());
    };
    let allowed: &[&str] = match action {
        "list" => &[],
        "register" => &[
            "--name",
            "--root",
            "--settings-digest",
            "--execution-policy",
            "--git-dir",
            "--common-dir",
        ],
        "rebind" => &[
            "--id",
            "--expected-revision",
            "--name",
            "--root",
            "--settings-digest",
            "--execution-policy",
            "--git-dir",
            "--common-dir",
        ],
        "revoke" => &["--id", "--expected-revision"],
        "grants" => &["--id"],
        "grant" => &[
            "--id",
            "--expected-revision",
            "--principal-kind",
            "--principal-id",
            "--permissions",
        ],
        "ungrant" => &[
            "--id",
            "--expected-revision",
            "--principal-kind",
            "--principal-id",
        ],
        "check" => &["--id", "--principal-kind", "--principal-id", "--permission"],
        _ => return Err(USAGE.into()),
    };
    if !(args.len() - 1).is_multiple_of(2) {
        return Err(USAGE.into());
    }
    let mut flags = BTreeMap::new();
    for pair in args[1..].as_chunks::<2>().0 {
        let name = pair[0].as_str();
        if !["--config", "--state-dir"].contains(&name) && !allowed.contains(&name)
            || flags.insert(name, pair[1].as_str()).is_some()
        {
            return Err("Unknown or repeated worker export flag".into());
        }
    }
    let config = crate::worker_client::load_config(Path::new(required(&flags, "--config")?))
        .await
        .map_err(str::to_owned)?;
    let scope = ExportScope {
        controller_node_id: config.controller.node_id,
        worker_node_id: config.network.node_id,
        workspace_id: config.workspace_id,
        host_id: config.host_id,
        enrollment_epoch: config.enrollment_epoch,
        credential_revision: config.credential_revision,
    };
    let store = WorkerStore::open(Path::new(required(&flags, "--state-dir")?), scope.clone())
        .map_err(|e| e.to_string())?;
    let value=(||->Result<serde_json::Value>{
        let req=|name|->Result<&str>{required(&flags,name).map_err(|_|Error::Invalid("Missing explicit worker export operation flag"))};
        let id=||req("--id");
        let principal=||principal(&flags).map_err(|_|Error::Invalid("Invalid explicit remote principal"));
        let expected=||expected(&flags).map_err(|_|Error::Invalid("Invalid expected worker export revision"));
        match action {
            "list"=>Ok(serde_json::json!({"exports":store.list(&scope)?})),
            "register"|"rebind"=>{
                let linked=match(flags.get("--git-dir"),flags.get("--common-dir")) {(None,None)=>None,(Some(git),Some(common))=>Some(LinkedRoots{git_dir:(*git).into(),common_dir:(*common).into()}),_=>return Err(Error::Invalid("Both explicit Git metadata roots are required together"))};
                let policy=match(flags.get("--execution-policy"),flags.get("--settings-digest")) {
                    (Some(path),None)=>Some(policy::PolicySnapshot::load(Path::new(path))?),
                    (None,Some(_))=>None,
                    _=>return Err(Error::Invalid("Exactly one of --execution-policy and --settings-digest is required")),
                };
                let digest=match &policy {Some(policy)=>policy.digest(),None=>req("--settings-digest")?};
                let checkout=CapturedCheckout::capture(Path::new(req("--root")?),digest,linked.as_ref())?;
                let summary=if action=="register"{store.register_captured_with_policy(&scope,req("--name")?,&checkout,policy.as_ref())?}else{store.rebind_captured_with_policy(&scope,id()?,expected()?,req("--name")?,&checkout,policy.as_ref())?};Ok(serde_json::to_value(summary)?)
            },
            "revoke"=>Ok(serde_json::to_value(store.revoke_export(&scope,id()?,expected()?)?)?),
            "grants"=>Ok(serde_json::json!({"grants":store.grants(&scope,id()?)?})),
            "grant"|"ungrant"=>{
                let p=principal()?;let revision=expected()?;
                let permissions=if action=="grant"{req("--permissions")?.split(',').map(permission).collect::<std::result::Result<Vec<_>,_>>().map_err(|_|Error::Invalid("Invalid export permissions"))?}else{vec![]};
                Ok(serde_json::to_value(store.set_grant(&scope,id()?,&p,&permissions,action=="grant",(revision!=0).then_some(revision))?)?)
            },
            "check"=>{
                let p=principal()?;let permission=permission(req("--permission")?).map_err(|_|Error::Invalid("Invalid export permission"))?;
                let candidate=store.candidate(&scope,id()?,&p,permission)?;
                let validated=candidate.validate_filesystem()?;
                let authority=store.authorize_validated(&scope,validated)?;
                authority.validate_filesystem()?;store.revalidate_authority(&scope,&authority)?;
                Ok(serde_json::json!({"checkout_identity_current":true,"grant_current":true,"execution_ready":false,"export":authority.export_stamp(),"grant":authority.grant_stamp()}))
            },_=>Err(Error::Invalid("Unsupported worker export operation")),
        }
    })().map_err(|e|e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|_| "Worker export output unavailable")?
    );
    Ok(())
}
