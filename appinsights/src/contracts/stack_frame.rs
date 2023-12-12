use crate::contracts::*;
use serde::Serialize;

// NOTE: This file was automatically generated.

/// Stack frame information.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    pub level: i32,
    pub method: String,
    pub assembly: Option<String>,
    pub file_name: Option<String>,
    pub line: Option<i32>,
}
