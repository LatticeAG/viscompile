use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use thiserror::Error;

pub const TRANSCRIPT_KIND: &str = "latticeag.viscompile.transcript";
pub const SNAPSHOT_KIND: &str = "latticeag.viscompile.snapshot";
pub const DIFF_KIND: &str = "latticeag.viscompile.diff";
pub const COMPARISON_KIND: &str = "latticeag.viscompile.comparison";
pub const COMPARISON_PROFILE: &str = "latticeag.viscompile/v1";
pub const SCHEMA_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, VisCompileError>;

/// Render untrusted input visibly in a terminal diagnostic or text report.
///
/// ```
/// assert_eq!(viscompile::escape_untrusted_text("a\nb"), "a\\nb");
/// ```
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
            [] => formatter.write_str("JSON Schema validation failed"),
            [violation] => write!(formatter, "JSON Schema validation failed: {violation}"),
            violations => {
                writeln!(
                    formatter,
                    "JSON Schema validation failed ({} violations):",
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
    #[error("VC1101: I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("VC1102: JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("VC1013: {0}")]
    TranscriptSchema(#[from] TranscriptSchemaValidationError),
    #[error("VC1901: internal transcript schema error: {0}")]
    SchemaDefinition(String),
    #[error("{code}: {message}")]
    Validation { code: &'static str, message: String },
}

impl VisCompileError {
    pub fn validation_code(code: &'static str, message: impl Into<String>) -> Self {
        Self::Validation {
            code,
            message: message.into(),
        }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Unchanged,
    Added,
    Removed,
    Changed,
    Improved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaseDiff {
    pub id: String,
    pub status: CaseStatus,
    pub regression: bool,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiffSummary {
    pub unchanged: u64,
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
    pub regressions: u64,
    pub improvements: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiffReport {
    pub kind: String,
    pub schema_version: u32,
    pub summary: DiffSummary,
    pub cases: Vec<CaseDiff>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WhyCode {
    CaseAdded,
    CaseRemoved,
    ErrorRaised,
    ErrorResolved,
    InputChanged,
    ToolCallReordered,
    ToolCallRemoved,
    ToolCallAdded,
    ToolCallArgumentsChanged,
    OutputChanged,
    ErrorChanged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOperation {
    Add,
    Remove,
    Replace,
    TypeChange,
}

impl ChangeOperation {
    pub fn rank(self) -> u8 {
        match self {
            Self::Remove => 0,
            Self::Add => 1,
            Self::Replace => 2,
            Self::TypeChange => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Absent,
    Present,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueSlot {
    pub presence: Presence,
    pub value: Option<Value>,
}

impl ValueSlot {
    pub fn absent() -> Self {
        Self {
            presence: Presence::Absent,
            value: None,
        }
    }

    pub fn present(value: Value) -> Self {
        Self {
            presence: Presence::Present,
            value: Some(value),
        }
    }
}

impl Serialize for ValueSlot {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        match self.presence {
            Presence::Absent => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("presence", "absent")?;
                map.end()
            }
            Presence::Present => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("presence", "present")?;
                map.serialize_entry("value", self.value.as_ref().unwrap_or(&Value::Null))?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ValueSlot {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("ValueSlot must be a JSON object"))?;
        for key in object.keys() {
            if key != "presence" && key != "value" {
                return Err(D::Error::unknown_field(key, &["presence", "value"]));
            }
        }
        let presence = match object.get("presence").and_then(Value::as_str) {
            Some("absent") => Presence::Absent,
            Some("present") => Presence::Present,
            Some(other) => {
                return Err(D::Error::unknown_variant(other, &["absent", "present"]));
            }
            None => return Err(D::Error::missing_field("presence")),
        };
        match presence {
            Presence::Absent => {
                if object.contains_key("value") {
                    return Err(D::Error::custom("absent ValueSlot must omit the value key"));
                }
                Ok(Self::absent())
            }
            Presence::Present => {
                let inner = object
                    .get("value")
                    .cloned()
                    .ok_or_else(|| D::Error::missing_field("value"))?;
                Ok(Self::present(inner))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChangeRecord {
    pub path: String,
    pub operation: ChangeOperation,
    pub before: ValueSlot,
    pub after: ValueSlot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentRelation {
    Equal,
    Replace,
    Remove,
    Insert,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolCallAlignment {
    pub baseline_index: Option<u64>,
    pub target_index: Option<u64>,
    pub relation: AlignmentRelation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaseComparison {
    pub id: String,
    pub status: CaseStatus,
    pub regression: bool,
    pub changed_fields: Vec<String>,
    pub why_codes: Vec<WhyCode>,
    pub changes: Vec<ChangeRecord>,
    pub tool_call_alignment: Vec<ToolCallAlignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonReport {
    pub kind: String,
    pub schema_version: u32,
    pub profile: String,
    pub baseline_digest: String,
    pub target_digest: String,
    pub summary: DiffSummary,
    pub cases: Vec<CaseComparison>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_report_rejects_unknown_fields() {
        let value = json!({
            "kind": DIFF_KIND,
            "schema_version": SCHEMA_VERSION,
            "summary": {
                "unchanged": 0,
                "added": 0,
                "removed": 0,
                "changed": 0,
                "regressions": 0,
                "improvements": 0
            },
            "cases": [],
            "extra": true
        });
        let error = serde_json::from_value::<DiffReport>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn value_slot_present_null_round_trips() {
        let slot = ValueSlot::present(Value::Null);
        let encoded = serde_json::to_value(&slot).expect("serialize");
        assert_eq!(encoded["presence"], "present");
        assert!(encoded.get("value").is_some());
        assert!(encoded["value"].is_null());
        let decoded: ValueSlot = serde_json::from_value(encoded).expect("deserialize");
        assert_eq!(decoded.presence, Presence::Present);
        assert_eq!(decoded.value, Some(Value::Null));

        let absent = serde_json::to_value(ValueSlot::absent()).expect("serialize absent");
        assert_eq!(absent["presence"], "absent");
        assert!(absent.get("value").is_none());
        let decoded_absent: ValueSlot = serde_json::from_value(absent).expect("deserialize absent");
        assert_eq!(decoded_absent, ValueSlot::absent());
    }
}
