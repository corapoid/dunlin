//! Host-attested CLI target and one-use permit, independent of HTTP admission.
use super::*;
use crate::recipe::{
    RecipeExecutor, RecipeInput, RecipeOutput, RecipePlan, RemoteRecipeExecutor, RemoteRecipePlan,
};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

mod target;
pub use target::{RecipeTarget, RemoteRecipeTarget};

/// HTTP authority alone never authorizes running a process on a host.
pub trait RecipeAuthority: Authority {
    fn check_target(
        &self,
        scope: &UseScope,
        version: &Version,
        target: &RecipeTarget,
    ) -> Result<()>;
}
struct Checked {
    authority: Arc<dyn RecipeAuthority>,
    target: RecipeTarget,
}
impl Authority for Checked {
    fn check(&self, scope: &UseScope, version: &Version) -> Result<()> {
        self.authority.check(scope, version)?;
        self.authority.check_target(scope, version, &self.target)
    }
    fn consume(&self, scope: &UseScope, version: &Version, digest: &str) -> Result<()> {
        self.check(scope, version)?;
        self.authority.consume(scope, version, digest)
    }
    fn complete(&self, scope: &UseScope, version: &Version, outcome: Completion) -> Result<()> {
        self.authority.complete(scope, version, outcome)
    }
}
/// A recipe permit cannot be decoded or substituted for an HTTP permit.
/// ```compile_fail
/// use dunlin_vault::scoped::RecipePermit;
/// let _: RecipePermit = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use dunlin_vault::scoped::RecipePermit;
/// fn duplicate(p: RecipePermit) -> (RecipePermit, RecipePermit) { (p.clone(), p) }
/// ```
pub struct RecipePermit {
    permit: Permit,
    target: RecipeTarget,
}
fn input_digest(input: &RecipeInput, target: &RecipeTarget) -> Result<String> {
    if !token(&input.secret)
        || input.recipe.is_empty()
        || input.recipe.len() > 64
        || !input
            .recipe
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
        || input.params.len() > 16
        || input.params.iter().any(|(key, value)| {
            key.len() > 48
                || key.is_empty()
                || value.len() > 4096
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return Err(Error::Invalid);
    }
    let raw = Zeroizing::new(
        serde_json::to_vec(&target.digest_input(input)).map_err(|_| Error::Invalid)?,
    );
    if raw.len() > 65536 {
        return Err(Error::Invalid);
    }
    Ok(digest(&raw))
}
enum Executor<'a> {
    Local(&'a dyn RecipeExecutor),
    Remote(&'a dyn RemoteRecipeExecutor),
}
impl<J: Journal> Credentials<J> {
    pub fn permit_recipe(
        &self,
        scope: UseScope,
        version: Version,
        input: &RecipeInput,
        target: RecipeTarget,
        deadline: Instant,
        authority: Arc<dyn RecipeAuthority>,
    ) -> Result<RecipePermit> {
        if scope.read_only || *target.binding() != scope.binding {
            return Err(Error::Denied);
        }
        let input_sha256 = input_digest(input, &target)?;
        let checked = Arc::new(Checked {
            authority,
            target: target.clone(),
        });
        let permit = self.permit_for(
            scope,
            version,
            &input.secret,
            input_sha256,
            deadline,
            checked,
        )?;
        Ok(RecipePermit { permit, target })
    }
    pub fn recipe(
        &self,
        permit: RecipePermit,
        input: RecipeInput,
        executor: &dyn RecipeExecutor,
    ) -> Result<RecipeOutput> {
        self.finish_recipe(permit, input, Executor::Local(executor))
    }
    pub fn recipe_remote(
        &self,
        permit: RecipePermit,
        input: RecipeInput,
        executor: &dyn RemoteRecipeExecutor,
    ) -> Result<RecipeOutput> {
        self.finish_recipe(permit, input, Executor::Remote(executor))
    }
    fn finish_recipe(
        &self,
        permit: RecipePermit,
        input: RecipeInput,
        executor: Executor<'_>,
    ) -> Result<RecipeOutput> {
        let mut dispatched = false;
        let result = self.recipe_inner(&permit, input, executor, &mut dispatched);
        let outcome = match &result {
            Ok(_) => Completion::Complete,
            Err(Error::OutcomeUnknown) => Completion::Unknown,
            Err(_) => Completion::Failed,
        };
        let permit = &permit.permit;
        if permit
            .authority
            .complete(&permit.scope, &permit.version, outcome)
            .is_err()
        {
            return Err(Error::OutcomeUnknown);
        }
        if !permit.access.allowed() {
            return Err(if dispatched {
                Error::OutcomeUnknown
            } else {
                Error::Denied
            });
        }
        result
    }
    fn recipe_inner(
        &self,
        ticket: &RecipePermit,
        input: RecipeInput,
        executor: Executor<'_>,
        dispatched: &mut bool,
    ) -> Result<RecipeOutput> {
        let permit = &ticket.permit;
        if input_digest(&input, &ticket.target)? != permit.input_sha256 || !permit.access.allowed()
        {
            return Err(Error::Denied);
        }
        if matches!(&executor, Executor::Local(_)) != ticket.target.local_cwd().is_some() {
            return Err(Error::Denied);
        }
        let record = self.load_record(permit)?;
        let mut matching = record
            .policy
            .recipes
            .iter()
            .filter(|recipe| recipe.name == input.recipe);
        let recipe = matching.next().ok_or(Error::Denied)?;
        if matching.next().is_some() {
            return Err(Error::Denied);
        }
        // Managed CLI credentials require an exact executable pin. Legacy recipes
        // retain their separate policy; a missing/malformed pin cannot dispatch.
        if recipe
            .program_hash
            .as_ref()
            .is_none_or(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(Error::Denied);
        }
        let args = recipe
            .render_args(&input.params)
            .map_err(|_| Error::Denied)?;
        let timeout = Duration::from_secs(recipe.timeout_secs).min(permit.access.remaining());
        if !permit.access.allowed() || timeout.is_zero() {
            return Err(Error::Denied);
        }
        let response = match executor {
            Executor::Local(executor) => {
                let plan = RecipePlan {
                    program: recipe.program.clone(),
                    args,
                    env_var: recipe.env_var.clone(),
                    secret: Zeroizing::new(record.value.clone()),
                    cwd: ticket.target.local_cwd().ok_or(Error::Denied)?.into(),
                    timeout,
                    program_hash: recipe.program_hash.clone(),
                };
                if !permit.access.allowed() {
                    return Err(Error::Denied);
                }
                *dispatched = true;
                executor.execute_guarded(&plan, Some(permit.access.clone()))
            }
            Executor::Remote(executor) => {
                let plan = RemoteRecipePlan {
                    target: ticket.target.remote_scope().ok_or(Error::Denied)?.clone(),
                    template: dunlin_domain::vault::RecipeTemplate::from_recipe(recipe)
                        .map_err(|_| Error::Denied)?,
                    params: input.params.clone(),
                    secret: Zeroizing::new(record.value.clone()),
                    timeout,
                    basic_username: match &record.policy.injection {
                        Injection::Basic { username } => Some(username.clone()),
                        _ => None,
                    },
                };
                if !permit.access.allowed() {
                    return Err(Error::Denied);
                }
                *dispatched = true;
                executor.execute_guarded(&plan, permit.access.clone())
            }
        };
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        let output = crate::recipe::redact_output(
            response.map_err(|_| Error::OutcomeUnknown)?,
            &record.value,
            match &record.policy.injection {
                Injection::Basic { username } => Some(username.as_str()),
                _ => None,
            },
        )
        .map_err(|_| Error::OutcomeUnknown)?;
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
