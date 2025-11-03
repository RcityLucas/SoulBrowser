use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub provider: Option<ProviderSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Permissions {
    #[serde(default)]
    pub net: Vec<String>,
    #[serde(default)]
    pub kv: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSpec {
    pub kind: String,
    pub tool: Option<String>,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty() {
            return Err(ManifestError::invalid("name"));
        }
        if self.version.is_empty() {
            return Err(ManifestError::invalid("version"));
        }
        if self.entry.is_empty() {
            return Err(ManifestError::invalid("entry"));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("missing field: {0}")]
    Missing(&'static str),
    #[error("invalid field: {0}")]
    Invalid(&'static str),
}

impl ManifestError {
    fn invalid(field: &'static str) -> Self {
        Self::Invalid(field)
    }
}
