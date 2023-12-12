use crate::contracts::*;
use serde::Serialize;

// NOTE: This file was automatically generated.

/// Exception details of the exception in a chain.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDetails {
    pub id: Option<i32>,
    pub outer_id: Option<i32>,
    pub type_name: String,
    pub message: String,
    pub has_full_stack: Option<bool>,
    pub stack: Option<String>,
    pub parsed_stack: Vec<StackFrame>,
}

impl Default for ExceptionDetails {
    fn default() -> Self {
        Self {
            id: Option::default(),
            outer_id: Option::default(),
            type_name: String::default(),
            message: String::default(),
            has_full_stack: Some(true),
            stack: Option::default(),
            parsed_stack: Vec::default(),
        }
    }
}
