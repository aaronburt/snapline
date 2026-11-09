use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflows {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Workflow>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflow {
    #[serde(flatten)]
    pub extra: toml::Table,
}
