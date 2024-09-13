// TODO implement exception collection telemetry item

use chrono::{DateTime, Utc};

use crate::{
    contracts::{Base, Data, Envelope, ExceptionData, ExceptionDetails},
    telemetry::{ContextTags, Measurements, Properties, SeverityLevel, Telemetry},
    time, TelemetryContext,
};

/// Represents errors that occur during application execution.
///
/// # Examples
/// ```rust, no_run
/// # use appinsights::TelemetryClient;
/// # let client = TelemetryClient::new("<instrumentation key>".to_string());
/// use appinsights::telemetry::{Telemetry, ExceptionTelemetry};
///
/// // create a telemetry item
/// let mut telemetry = ExceptionTelemetry::new(
///     "File does not exist",
///     Some(SeverityLevel::Error),
///     Some(format!{"FileNotFound: {}:{}:{}", file!(), line!(), column!()})
/// )
/// .with_message("File does not exist", "FileNotFound", None);
///
/// // attach custom properties, measurements and context tags
/// telemetry.properties_mut().insert("component".to_string(), "data_processor".to_string());
/// telemetry.tags_mut().insert("os_version".to_string(), "linux x86_64".to_string());
/// telemetry.measurements_mut().insert("body_size".to_string(), 115.0);
///
/// // submit telemetry item to server
/// client.track(telemetry);
/// ```
#[derive(Debug)]
pub struct ExceptionTelemetry {
    /// Exception chain - list of inner exceptions.
    /// TODO: should this be a Vec? Currently only one exception, not exceptions.
    exceptions: Vec<ExceptionDetails>,

    /// Severity level. Mostly used to indicate exception severity level
    /// when it is reported by logging library.
    severity_level: Option<SeverityLevel>,

    /// Identifier of where the exception was thrown in code.
    /// Used for exceptions grouping. Typically a combination of exception type
    /// and a function from the call stack.
    problem_id: Option<String>,

    /// Collection of custom properties.
    properties: Properties,

    /// Collection of custom measurements.
    measurements: Measurements,

    /// The time stamp when this telemetry was measured.
    timestamp: DateTime<Utc>,

    /// Telemetry context containing extra, optional tags.
    tags: ContextTags,
}

impl ExceptionTelemetry {
    /// Create an empty exception telemetry item.
    pub fn new(severity_level: Option<SeverityLevel>, problem_id: Option<impl Into<String>>) -> Self {
        Self {
            exceptions: vec![],
            severity_level,
            problem_id: problem_id.map(|id| id.into()),
            timestamp: time::now(),
            properties: Properties::default(),
            measurements: Measurements::default(),
            tags: ContextTags::default(),
        }
    }

    /// Add a new exception with the given parameters to the list of exceptions
    /// of this exception telemetry item.
    ///
    /// ### Bugs
    /// Adding multiple exceptions to a single telemetry item does not
    /// actually work yet. The nesting is not shown in Azure App Insights,
    /// and messages of the exceptions are just concatenated.
    pub fn with_message(
        mut self,
        message: impl Into<String>,
        type_name: impl Into<String>,
        stack_trace: Option<impl Into<String>>,
    ) -> Self {
        self.exceptions.push(ExceptionDetails {
            message: message.into(),
            type_name: type_name.into(),
            stack: stack_trace.map(|s| s.into()),
            ..Default::default()
        });
        self
    }

    /// Add an exception to the list of exceptions of this exception
    /// telemetry item.
    ///
    /// ### Bugs
    /// Adding multiple exceptions to a single telemetry item does not
    /// actually work yet.
    pub fn with_exception(mut self, exception: ExceptionDetails) -> Self {
        self.exceptions.push(exception);
        self
    }

    /// Returns mutable reference to the timestamp.
    pub fn timestamp_mut(&mut self) -> &mut DateTime<Utc> {
        &mut self.timestamp
    }

    /// Create a new [ExceptionTelemetryBuilder], used to construct an [ExceptionTelemetry].
    pub fn builder() -> ExceptionTelemetryBuilder {
        ExceptionTelemetryBuilder::default()
    }
}

impl Telemetry for ExceptionTelemetry {
    fn properties(&self) -> &Properties {
        &self.properties
    }

    fn properties_mut(&mut self) -> &mut Properties {
        &mut self.properties
    }

    fn tags(&self) -> &ContextTags {
        &self.tags
    }

    fn tags_mut(&mut self) -> &mut ContextTags {
        &mut self.tags
    }

    fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }
}

impl From<(TelemetryContext, ExceptionTelemetry)> for Envelope {
    fn from((context, telemetry): (TelemetryContext, ExceptionTelemetry)) -> Self {
        Self {
            name: "Microsoft.ApplicationInsights.Exception".into(),
            time: telemetry.timestamp.to_rfc3339_opts(context.timestamp_format, true),
            i_key: Some(context.i_key),
            tags: Some(ContextTags::combine(context.tags, telemetry.tags).into()),
            data: Some(Base::Data(Data::ExceptionData(ExceptionData {
                exceptions: telemetry.exceptions,
                problem_id: telemetry.problem_id,
                severity_level: telemetry.severity_level.map(|s| s.into()),
                properties: Some(Properties::combine(context.properties, telemetry.properties).into()),
                measurements: Some(telemetry.measurements.into()),
                ..Default::default()
            }))),
            ..Default::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct ExceptionTelemetryBuilder {
    exceptions: Vec<ExceptionDetails>,
    severity_level: Option<SeverityLevel>,
    problem_id: Option<String>,
    properties: Option<Properties>,
    measurements: Option<Measurements>,
    timestamp: Option<DateTime<Utc>>,
    tags: Option<ContextTags>,
}

impl ExceptionTelemetryBuilder {
    pub fn with_severity(mut self, severity: SeverityLevel) -> Self {
        self.severity_level = Some(severity);
        self
    }

    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.properties = Some(properties);
        self
    }

    pub fn with_tags(mut self, tags: ContextTags) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn with_measurements(mut self, measurements: Measurements) -> Self {
        self.measurements = Some(measurements);
        self
    }

    pub fn with_problem_id(mut self, problem_id: impl Into<String>) -> Self {
        self.problem_id = Some(problem_id.into());
        self
    }

    /// Can be called multiple times to add several exceptions to the exception
    /// chain of the `ExceptionTelemetry`.
    ///
    /// ### Bugs
    /// Adding multiple exceptions to a single telemetry item does not
    /// actually work yet.
    pub fn with_exception(mut self, exception: ExceptionDetails) -> Self {
        self.exceptions.push(exception);
        self
    }

    pub fn build(self) -> ExceptionTelemetry {
        ExceptionTelemetry {
            severity_level: self.severity_level,
            exceptions: self.exceptions,
            timestamp: self.timestamp.unwrap_or_else(time::now),
            properties: self.properties.unwrap_or_default(),
            tags: self.tags.unwrap_or_default(),
            measurements: self.measurements.unwrap_or_default(),
            problem_id: self.problem_id,
        }
    }
}
