use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub input: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuite {
    pub tests: Vec<TestCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String, // JSON string
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub input: String,
    pub output_text: String,
    pub tool_calls_made: Vec<ToolCall>,
    pub latency_ms: u64,
    pub estimated_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub model_used: String,
    pub prompt_version_hash: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub metadata: Metadata,
    pub results: Vec<TestResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestDiff {
    pub input: String,
    pub output_changed: bool,
    pub tool_calls_changed: bool,
    pub latency_delta: i64,
    pub cost_delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedStats {
    pub total_regressions: u64,
    pub total_improvements: u64,
    pub unchanged: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub test_diffs: Vec<TestDiff>,
    pub stats: AggregatedStats,
    pub summary: String,
}
