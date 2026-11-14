//! Host-attested CLI target and one-use permit, independent of HTTP admission.
use super::*;
use crate::recipe::{RecipeExecutor, RecipeInput, RecipeOutput, RecipePlan};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Shape only, not an authority proof. The host must attest this exact binding,
/// canonical cwd and immutable checkout stamp on every RecipeAuthority check.
/// This type is never deserialized from model arguments or a runner request.
#[derive(Clone, PartialEq, Eq)]
pub struct RecipeTarget {
    binding: Binding,
    cwd: PathBuf,
    checkout_stamp: String,
}
impl RecipeTarget {
    pub fn new(binding: Binding, cwd: PathBuf, checkout_stamp: String) -> Result<Self> {
        binding.validate()?;
        if !cwd.is_absolute()
            || cwd.as_os_str().len() > 4096
            || cwd
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || checkout_stamp.len() != 64
            || !checkout_stamp
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            binding,
            cwd,
            checkout_stamp,
        })
    }
    pub fn input_digest(&self, input: &RecipeInput) -> Result<String> {
        input_digest(input, self)
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn checkout_stamp(&self) -> &str {
        &self.checkout_stamp
    }
}
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
        serde_json::to_vec(&serde_json::json!({
            "domain":"dunlin.scoped-recipe.v1", "input":input,
            "binding":target.binding, "cwd":target.cwd, "checkout_stamp":target.checkout_stamp
        }))
        .map_err(|_| Error::Invalid)?,
    );
    if raw.len() > 65536 {
        return Err(Error::Invalid);
    }
    Ok(digest(&raw))
}
fn cap(value: &mut String) -> bool {
    if value.len() <= http::MAX_RESPONSE_BODY {
        return false;
    }
    let mut end = http::MAX_RESPONSE_BODY;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    true
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
        if scope.read_only || target.binding != scope.binding {
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
        executor: &dyn RecipeExecutor,
        dispatched: &mut bool,
    ) -> Result<RecipeOutput> {
        let permit = &ticket.permit;
        if input_digest(&input, &ticket.target)? != permit.input_sha256 || !permit.access.allowed()
        {
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
        let plan = RecipePlan {
            program: recipe.program.clone(),
            args,
            env_var: recipe.env_var.clone(),
            secret: Zeroizing::new(record.value.clone()),
            cwd: ticket.target.cwd.clone(),
            timeout: Duration::from_secs(recipe.timeout_secs).min(permit.access.remaining()),
            program_hash: recipe.program_hash.clone(),
        };
        if !permit.access.allowed() || plan.timeout.is_zero() {
            return Err(Error::Denied);
        }
        *dispatched = true;
        let response = executor.execute_guarded(&plan, Some(permit.access.clone()));
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        let mut output = response.map_err(|_| Error::OutcomeUnknown)?;
        let stdout = Zeroizing::new(std::mem::take(&mut output.stdout));
        let stderr = Zeroizing::new(std::mem::take(&mut output.stderr));
        if stdout.len() > http::MAX_RESPONSE_BODY || stderr.len() > http::MAX_RESPONSE_BODY {
            return Err(Error::OutcomeUnknown);
        }
        let basic = match &record.policy.injection {
            Injection::Basic { username } => {
                Some(Zeroizing::new(format!("{username}:{}", record.value)))
            }
            _ => None,
        };
        let basic = basic.as_ref().map(|value| value.as_str());
        output.stdout = http::redact(&stdout, &record.value, basic);
        output.stderr = http::redact(&stderr, &record.value, basic);
        output.truncated |= cap(&mut output.stdout) | cap(&mut output.stderr);
        if !permit.access.allowed() {
            return Err(Error::OutcomeUnknown);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
