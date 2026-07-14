use serde::Deserialize;

const MAX_STATUS_BYTES: u64 = 16 * 1024;
const MAX_ITEMS_PER_PLUGIN: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginStatusItem {
    pub plugin_id: String,
    pub id: String,
    pub label: String,
    pub severity: PluginStatusSeverity,
    pub priority: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PluginStatusSeverity {
    Normal,
    Warning,
}

#[derive(Deserialize)]
struct StatusDocument {
    items: Vec<StatusDocumentItem>,
}

#[derive(Deserialize)]
struct StatusDocumentItem {
    id: String,
    label: String,
    #[serde(default = "normal_severity")]
    severity: PluginStatusSeverity,
    #[serde(default)]
    priority: i32,
}

fn normal_severity() -> PluginStatusSeverity {
    PluginStatusSeverity::Normal
}

pub(crate) fn load(
    installed: &crate::app::state::InstalledPluginRegistry,
) -> Vec<PluginStatusItem> {
    let mut items = installed
        .values()
        .filter(|plugin| plugin.enabled)
        .flat_map(|plugin| load_plugin(&plugin.plugin_id))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.cmp(&right.plugin_id))
            .then_with(|| left.id.cmp(&right.id))
    });
    items
}

fn load_plugin(plugin_id: &str) -> Vec<PluginStatusItem> {
    let path = crate::plugin_paths::plugin_state_dir(plugin_id).join("status.json");
    let Ok(metadata) = std::fs::metadata(&path) else {
        return Vec::new();
    };
    if metadata.len() > MAX_STATUS_BYTES {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_str::<StatusDocument>(&content) else {
        return Vec::new();
    };
    document
        .items
        .into_iter()
        .take(MAX_ITEMS_PER_PLUGIN)
        .filter(|item| valid_token(&item.id) && valid_label(&item.label))
        .map(|item| PluginStatusItem {
            plugin_id: plugin_id.to_string(),
            id: item.id,
            label: item.label,
            severity: item.severity,
            priority: item.priority,
        })
        .collect()
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 80 && !value.chars().any(char::is_control)
}
