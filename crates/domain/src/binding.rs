//! Explicit project target references. Neither a reference nor validation of its
//! syntax grants access to a checkout, host or process.
use serde::{Deserialize, Serialize};
pub const MAX_BINDING_REVISION: u64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingRef {
    pub id: String,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectTargetRef {
    Local { revision: u64 },
    Remote { binding: BindingRef },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindingValidationError;
impl std::fmt::Display for BindingValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("A bounded binding ID and positive safe integer revision are required")
    }
}
impl std::error::Error for BindingValidationError {}
fn revision(value: u64) -> Result<(), BindingValidationError> {
    if (1..=MAX_BINDING_REVISION).contains(&value) {
        Ok(())
    } else {
        Err(BindingValidationError)
    }
}
impl BindingRef {
    pub fn validate(&self) -> Result<(), BindingValidationError> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(BindingValidationError);
        }
        revision(self.revision)
    }
}
impl ProjectTargetRef {
    pub fn validate(&self) -> Result<(), BindingValidationError> {
        match self {
            Self::Local { revision: value } => revision(*value),
            Self::Remote { binding } => binding.validate(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn explicit_targets_reject_ambiguous_kinds_paths_fields_and_unsafe_revisions() {
        for value in [
            json!({"kind":"remote","binding":{"id":"rbd-1","revision":3}}),
            json!({"kind":"local","revision":2}),
        ] {
            let target: ProjectTargetRef = serde_json::from_value(value.clone()).unwrap();
            target.validate().unwrap();
            assert_eq!(serde_json::to_value(target).unwrap(), value);
        }
        for value in [
            json!({"kind":"other","revision":1}),
            json!({"kind":"local","revision":1,"host":"remote"}),
            json!({"kind":"remote","binding":{"id":"rbd-1","revision":1,"root":"/private"}}),
            json!({"kind":"remote","revision":1}),
        ] {
            assert!(serde_json::from_value::<ProjectTargetRef>(value).is_err());
        }
        for value in [0, MAX_BINDING_REVISION + 1, u64::MAX] {
            assert!(
                ProjectTargetRef::Local { revision: value }
                    .validate()
                    .is_err()
            );
            assert!(
                BindingRef {
                    id: "rbd-1".into(),
                    revision: value
                }
                .validate()
                .is_err()
            );
        }
        for id in ["".into(), "../root".into(), "a\nb".into(), "x".repeat(129)] {
            assert!(BindingRef { id, revision: 1 }.validate().is_err());
        }
    }
}
