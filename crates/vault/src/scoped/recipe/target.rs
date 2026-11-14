//! Validated configuration shapes only. A live RecipeAuthority must attest them.
use super::*;
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteRecipeTarget {
    binding: Binding,
    channel_stamp: String,
    turn_stamp: String,
    export_id: String,
    export_revision: u64,
    checkout_stamp: String,
}
fn stamp(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl RemoteRecipeTarget {
    /// These digests must come from a fresh trusted adapter's complete live
    /// channel/boot/turn and export/checkout proof. A wire DTO cannot attest it.
    pub fn new(
        binding: Binding,
        channel_stamp: String,
        turn_stamp: String,
        export_id: String,
        export_revision: u64,
        checkout_stamp: String,
    ) -> Result<Self> {
        binding.validate()?;
        if binding.host == "local"
            || !stamp(&channel_stamp)
            || !stamp(&turn_stamp)
            || !token(&export_id)
            || !(1..=9_007_199_254_740_991).contains(&export_revision)
            || !stamp(&checkout_stamp)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            binding,
            channel_stamp,
            turn_stamp,
            export_id,
            export_revision,
            checkout_stamp,
        })
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn channel_stamp(&self) -> &str {
        &self.channel_stamp
    }
    pub fn turn_stamp(&self) -> &str {
        &self.turn_stamp
    }
    pub fn export_id(&self) -> &str {
        &self.export_id
    }
    pub fn export_revision(&self) -> u64 {
        self.export_revision
    }
    pub fn checkout_stamp(&self) -> &str {
        &self.checkout_stamp
    }
}
#[derive(Clone, PartialEq, Eq)]
enum Location {
    Local {
        binding: Binding,
        cwd: PathBuf,
        checkout_stamp: String,
    },
    Remote(RemoteRecipeTarget),
}
/// No Deserialize. This shape is not evidence of live execution authority.
/// ```compile_fail
/// use dunlin_vault::scoped::RecipeTarget;
/// let _:RecipeTarget=serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct RecipeTarget(Location);
impl RecipeTarget {
    pub fn new(binding: Binding, cwd: PathBuf, checkout_stamp: String) -> Result<Self> {
        binding.validate()?;
        if !cwd.is_absolute()
            || cwd.as_os_str().len() > 4096
            || cwd
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || !stamp(&checkout_stamp)
        {
            return Err(Error::Invalid);
        }
        Ok(Self(Location::Local {
            binding,
            cwd,
            checkout_stamp,
        }))
    }
    pub fn from_remote(target: RemoteRecipeTarget) -> Self {
        Self(Location::Remote(target))
    }
    pub fn input_digest(&self, input: &RecipeInput) -> Result<String> {
        input_digest(input, self)
    }
    pub fn binding(&self) -> &Binding {
        match &self.0 {
            Location::Local { binding, .. } => binding,
            Location::Remote(t) => t.binding(),
        }
    }
    pub fn local_cwd(&self) -> Option<&Path> {
        match &self.0 {
            Location::Local { cwd, .. } => Some(cwd),
            Location::Remote(_) => None,
        }
    }
    pub fn remote_scope(&self) -> Option<&RemoteRecipeTarget> {
        match &self.0 {
            Location::Local { .. } => None,
            Location::Remote(t) => Some(t),
        }
    }
    pub fn checkout_stamp(&self) -> &str {
        match &self.0 {
            Location::Local { checkout_stamp, .. } => checkout_stamp,
            Location::Remote(t) => t.checkout_stamp(),
        }
    }
    pub(super) fn digest_input(&self, input: &RecipeInput) -> serde_json::Value {
        match &self.0 {
            // Preserve the established local approval fingerprint exactly.
            Location::Local {
                binding,
                cwd,
                checkout_stamp,
            } => serde_json::json!({
                "domain":"dunlin.scoped-recipe.v1","input":input,"binding":binding,"cwd":cwd,"checkout_stamp":checkout_stamp
            }),
            Location::Remote(t) => serde_json::json!({
                "domain":"dunlin.scoped-recipe.remote.v1","input":input,"binding":t.binding,
                "channel_stamp":t.channel_stamp,"turn_stamp":t.turn_stamp,"export_id":t.export_id,
                "export_revision":t.export_revision,"checkout_stamp":t.checkout_stamp
            }),
        }
    }
}
