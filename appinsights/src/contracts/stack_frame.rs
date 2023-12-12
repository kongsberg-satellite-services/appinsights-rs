use crate::contracts::*;
use serde::Serialize;

// NOTE: This file was automatically generated.

/// Stack frame information.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    level: i32,
    method: String,
    assembly: Option<String>,
    file_name: Option<String>,
    line: Option<i32>,
}
