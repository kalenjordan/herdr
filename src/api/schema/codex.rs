use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CodexThreadRenameCurrentParams {
    pub caller_pane_id: String,
    pub name: String,
}
