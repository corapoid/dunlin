//! A remote operation never contains a controller program path or cwd.
use super::*;
use crate::scoped::RemoteRecipeTarget;
use dunlin_domain::vault::RecipeTemplate;
use std::collections::BTreeMap;

/// Internal execution input, not a wire command, restored permit or authority.
/// ```compile_fail
/// use dunlin_vault::recipe::RemoteRecipePlan;
/// let _:RemoteRecipePlan=serde_json::from_str("{}").unwrap();
/// ```
pub struct RemoteRecipePlan {
    pub target: RemoteRecipeTarget,
    pub template: RecipeTemplate,
    pub params: BTreeMap<String, String>,
    pub secret: Zeroizing<String>,
    pub timeout: Duration,
    pub(crate) basic_username: Option<String>,
}
impl RemoteRecipePlan {
    /// Redact and bound both streams before any private-channel response or
    /// durable completion. The controller repeats redaction after receiving it.
    pub fn redact_output(&self, output: RecipeOutput) -> Result<RecipeOutput, String> {
        super::redact_output(output, &self.secret, self.basic_username.as_deref())
    }
}
impl std::fmt::Debug for RemoteRecipePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteRecipePlan")
            .field("recipe", &self.template.name)
            .field("parameter_count", &self.params.len())
            .field("secret", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .finish()
    }
}
/// Implemented only by a trusted private-channel adapter. There is deliberately
/// no local/default/unguarded fallback. The adapter must attest plan.target and
/// carry access through dispatch, process lifetime, redaction and final receipt.
pub trait RemoteRecipeExecutor: Send + Sync {
    fn execute_guarded(
        &self,
        plan: &RemoteRecipePlan,
        access: LiveAccess,
    ) -> Result<RecipeOutput, String>;
}
