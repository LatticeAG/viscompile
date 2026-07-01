use crate::types::{Metadata, Snapshot, TestResult, TestSuite};
use chrono::Utc;
use std::fs;
use std::path::Path;

pub fn run_snapshot(
    test_suite_path: &Path,
    out_path: &Path,
    model: &str,
    _endpoint: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let suite_data = fs::read_to_string(test_suite_path)?;
    let suite: TestSuite = serde_json::from_str(&suite_data)?;

    let mut results = Vec::new();

    for test in suite.tests {
        // Mock agent execution
        let result = TestResult {
            input: test.input.clone(),
            output_text: format!("Mocked output for: {}", test.input),
            tool_calls_made: vec![],
            latency_ms: 100,
            estimated_cost: 0.0001,
        };
        results.push(result);
    }

    let snapshot = Snapshot {
        metadata: Metadata {
            model_used: model.to_string(),
            prompt_version_hash: "mock_hash".to_string(),
            timestamp: Utc::now(),
        },
        results,
    };

    let snapshot_json = serde_json::to_string_pretty(&snapshot)?;
    fs::write(out_path, snapshot_json)?;

    println!("Snapshot saved to {}", out_path.display());

    Ok(())
}
