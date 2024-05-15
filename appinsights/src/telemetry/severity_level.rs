use crate::contracts::SeverityLevel as ContractsSeverityLevel;

/// Defines the level of severity for the event.
#[derive(Debug, Clone, Copy)]
pub enum SeverityLevel {
    /// Verbose severity level.
    Verbose,

    /// Information severity level.
    Information,

    /// Warning severity level.
    Warning,

    /// Error severity level.
    Error,

    /// Critical severity level.
    Critical,
}

impl From<SeverityLevel> for ContractsSeverityLevel {
    fn from(severity: SeverityLevel) -> Self {
        match severity {
            SeverityLevel::Verbose => ContractsSeverityLevel::Verbose,
            SeverityLevel::Information => ContractsSeverityLevel::Information,
            SeverityLevel::Warning => ContractsSeverityLevel::Warning,
            SeverityLevel::Error => ContractsSeverityLevel::Error,
            SeverityLevel::Critical => ContractsSeverityLevel::Critical,
        }
    }
}
