use crate::runtime::Runtime;
use serde::{Deserialize, Serialize};

fn default_category() -> String {
    "其他".into()
}
fn default_distribution() -> String {
    "bundled".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub kind: String,
    pub minimum: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub revision: String,
    pub source_url: String,
    pub sha256: String,
    pub size: u64,
    pub file_count: usize,
    pub package_path: String,
    pub requirements: Vec<Requirement>,
    pub markdown: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub setup_notes: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default = "default_distribution")]
    pub distribution: String,
    #[serde(default)]
    pub distribution_reason: Option<String>,
    /// Curated by the server; the client can offer a recommended-only view.
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub configuration: Option<crate::configuration::Flow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub python_dependencies: Vec<crate::python_dependencies::Profile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<crate::external_tools::Tool>,
    /// Shared support packages that must sit next to this Skill so its relative
    /// `../<id>/...` references resolve.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_dependencies: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub kind: String,
    pub version: String,
    pub platform: String,
    pub sha256: String,
    pub size: u64,
    pub path: String,
    pub source_url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGroupItem {
    pub skill_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGroup {
    pub id: String,
    pub title: String,
    pub description: String,
    pub source_url: String,
    pub items: Vec<SkillGroupItem>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema: u32,
    pub generated_at: String,
    pub skills: Vec<Skill>,
    pub runtimes: Vec<Artifact>,
    /// Absent in legacy catalogs; those Skills remain standalone items.
    #[serde(default)]
    pub skill_groups: Vec<SkillGroup>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageFile {
    pub path: String,
    pub size: usize,
    pub sha256: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPack {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub revision: String,
    pub requirements: Vec<Requirement>,
    pub files: Vec<PackageFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub skill_id: String,
    pub title: String,
    pub version: String,
    pub revision: String,
    pub target: String,
    pub scope: String,
    pub path: String,
    pub installed_at: u64,
    pub runtimes: Vec<Runtime>,
    pub package_sha256: String,
    #[serde(default)]
    pub configuration: Option<crate::configuration::Flow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub record: Record,
    pub exists: bool,
    pub managed: bool,
    pub removable: bool,
    pub removal_blocked_reason: Option<String>,
    /// Agents that discover this path. The record retains its original owner.
    pub visible_targets: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub server_url: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub action: String,
    pub operation_id: Option<String>,
    pub skill_id: Option<String>,
    pub target: Option<String>,
    pub scope: Option<String>,
    pub project_path: Option<String>,
    pub record_id: Option<String>,
    pub installation_path: Option<String>,
    pub values: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    pub flow_revision: Option<String>,
    pub codex: Option<crate::codex_config::Input>,
    pub claude: Option<crate::claude_config::Input>,
    pub opencode: Option<crate::opencode_config::Input>,
    pub configuration_revision: Option<String>,
    pub integration_id: Option<String>,
    pub workspace: Option<String>,
    pub server_id: Option<String>,
    pub action_id: Option<String>,
    pub url: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_api: Option<String>,
}
