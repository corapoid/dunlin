//! Secret-free semantic template shared by credential and runner policy.
//! Configuration integrity never grants current execution authority.
use super::{Recipe, RecipeParameter, valid_recipe_secret_env};
type Result<T> = std::result::Result<T, String>;
fn invalid(_path: &str, message: &str) -> String {
    message.into()
}
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateParameter {
    pub name: String,
    pub required: bool,
    pub max_len: usize,
    pub allow_leading_dash: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeTemplate {
    pub name: String,
    pub program_sha256: String,
    pub args: Vec<String>,
    pub env_var: String,
    pub parameters: Vec<TemplateParameter>,
    pub timeout_secs: u64,
}
impl RecipeTemplate {
    fn recipe(&self, program: &str) -> Recipe {
        Recipe {
            id: self.name.clone(),
            secret_id: "host-owned".into(),
            name: self.name.clone(),
            description: String::new(),
            program: program.into(),
            args: self.args.clone(),
            env_var: self.env_var.clone(),
            parameters: self
                .parameters
                .iter()
                .map(|p| RecipeParameter {
                    name: p.name.clone(),
                    required: p.required,
                    max_len: p.max_len,
                    allow_leading_dash: p.allow_leading_dash,
                })
                .collect(),
            timeout_secs: self.timeout_secs,
            program_hash: Some(self.program_sha256.clone()),
            created_at: 0,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.name.len() > 64
            || self.args.len() > 32
            || self.parameters.len() > 16
            || self.program_sha256.len() != 64
            || !self
                .program_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.env_var.len() > 128
            || !valid_recipe_secret_env(&self.env_var)
            || self
                .args
                .iter()
                .any(|a| a.len() > 4096 || a.chars().any(char::is_control))
            || self.parameters.iter().any(|p| p.name.len() > 48)
        {
            return Err(invalid(
                "recipe_template",
                "Invalid bounded recipe template",
            ));
        }
        self.recipe("/run/dunlin-recipe/program")
            .validate()
            .map_err(|_| invalid("recipe_template", "Invalid recipe template"))?;
        if serde_json::to_vec(self)
            .map_err(|_| invalid("recipe_template", "Invalid template"))?
            .len()
            > 16 * 1024
        {
            return Err(invalid("recipe_template", "Recipe template exceeds 16 KiB"));
        }
        Ok(())
    }
    /// Validate operator configuration, not filesystem identity or execution authority.
    pub fn validate_program(&self, program: &str) -> Result<()> {
        self.validate()?;
        if program.len() > 4096
            || !program.starts_with('/')
            || program
                .split('/')
                .skip(1)
                .any(|p| p.is_empty() || p == "." || p == "..")
            || program.chars().any(char::is_control)
        {
            return Err(invalid(
                "program",
                "A normalized absolute program path is required",
            ));
        }
        self.recipe(program)
            .validate()
            .map_err(|_| invalid("program", "Unsupported recipe program"))
    }
    pub fn from_recipe(recipe: &Recipe) -> Result<Self> {
        let template = Self {
            name: recipe.name.clone(),
            program_sha256: recipe.program_hash.clone().unwrap_or_default(),
            args: recipe.args.clone(),
            env_var: recipe.env_var.clone(),
            parameters: recipe
                .parameters
                .iter()
                .map(|p| TemplateParameter {
                    name: p.name.clone(),
                    required: p.required,
                    max_len: p.max_len,
                    allow_leading_dash: p.allow_leading_dash,
                })
                .collect(),
            timeout_secs: recipe.timeout_secs,
        };
        template.validate_program(&recipe.program)?;
        Ok(template)
    }
    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        let raw = serde_json::to_vec(&("dunlin.worker.recipe-template.v1", self))
            .map_err(|_| invalid("recipe_template", "Cannot encode template"))?;
        Ok(hex::encode(Sha256::digest(raw)))
    }
    /// Values stay individual argv elements; no shell interpretation is introduced.
    pub fn render_args(&self, params: &BTreeMap<String, String>) -> Result<Vec<String>> {
        self.validate()?;
        self.recipe("/run/dunlin-recipe/program")
            .render_args(params)
            .map_err(|_| invalid("params", "Recipe parameters do not match template"))
    }
}
#[cfg(test)]
mod tests;
