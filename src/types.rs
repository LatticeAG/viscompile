use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use thiserror::Error;

pub const TRANSCRIPT_KIND: &str = "latticeag.viscompile.transcript";
pub const SNAPSHOT_KIND: &str = "latticeag.viscompile.snapshot";
pub const DIFF_KIND: &str = "latticeag.viscompile.diff";
pub const SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, VisCompileError>;

/// Render untrusted input visibly in a terminal diagnostic or text report.
pub fn escape_untrusted_text(value: &str) -> String {
    value.escape_default().to_string()
}

/// One location-specific violation reported by the transcript JSON Schema.
///
/// The paths use JSON Pointer syntax. Keeping these details structured lets callers present
/// validation errors in a CLI, editor, or API without parsing a human-readable error string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    pub instance_path: String,
    pub schema_path: String,
    pub message: String,
}

impl Display for SchemaViolation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let instance_path = if self.instance_path.is_empty() {
            "/"
        } else {
            &self.instance_path
        };
        let schema_path = if self.schema_path.is_empty() {
            "/"
        } else {
            &self.schema_path
        };

        write!(
            formatter,
            "{instance_path}: {} (schema: {schema_path})",
            self.message
        )
    }
}

/// Aggregate schema validation failure for a raw transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSchemaValidationError {
    pub violations: Vec<SchemaViolation>,
}

impl TranscriptSchemaValidationError {
    pub fn new(mut violations: Vec<SchemaViolation>) -> Self {
        violations.sort_by(|left, right| {
            (
                left.instance_path.as_str(),
                left.schema_path.as_str(),
                left.message.as_str(),
            )
                .cmp(&(
                    right.instance_path.as_str(),
                    right.schema_path.as_str(),
                    right.message.as_str(),
                ))
        });
        violations.dedup();
        Self { violations }
    }
}

impl Display for TranscriptSchemaValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self.violations.as_slice() {
            [] => formatter.write_str("transcript JSON Schema validation failed"),
            [violation] => write!(
                formatter,
                "transcript JSON Schema validation failed: {violation}"
            ),
            violations => {
                writeln!(
                    formatter,
                    "transcript JSON Schema validation failed ({} violations):",
                    violations.len()
                )?;
                for violation in violations {
                    writeln!(formatter, "  - {violation}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for TranscriptSchemaValidationError {}

#[derive(Debug, Error)]
pub enum VisCompileError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation error: {0}")]
    TranscriptSchema(#[from] TranscriptSchemaValidationError),
    #[error("internal transcript schema error: {0}")]
    SchemaDefinition(String),
    #[error("validation error: {0}")]
    Validation(String),
}

impl VisCompileError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn schema_definition(message: impl Into<String>) -> Self {
        Self::SchemaDefinition(message.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalToolCall {
    pub name: String,
    pub arguments: Value,
}

/// Presence-aware canonical final output.
///
/// A transcript final may legitimately contain JSON `null`. A plain `Option<Value>` cannot
/// distinguish that from a missing `output` property when deserializing a persisted snapshot, so
/// this type records whether the property was present separately from its JSON value.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum CanonicalOutput {
    #[default]
    Absent,
    Value(Value),
}

impl CanonicalOutput {
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub fn is_present(&self) -> bool {
        !self.is_absent()
    }
}

impl Serialize for CanonicalOutput {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Absent => serializer.serialize_unit(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for CanonicalOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::Value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCase {
    pub id: String,
    pub input: Value,
    #[serde(default)]
    pub tool_calls: Vec<CanonicalToolCall>,
    #[serde(default, skip_serializing_if = "CanonicalOutput::is_absent")]
    pub output: CanonicalOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalSnapshot {
    pub kind: String,
    pub schema_version: u32,
    pub cases: Vec<CanonicalCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Unchanged,
    Added,
    Removed,
    Changed,
    Improved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaseDiff {
    pub id: String,
    pub status: CaseStatus,
    pub regression: bool,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffSummary {
    pub unchanged: u64,
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
    pub regressions: u64,
    pub improvements: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffReport {
    pub kind: String,
    pub schema_version: u32,
    pub summary: DiffSummary,
    pub cases: Vec<CaseDiff>,
}
