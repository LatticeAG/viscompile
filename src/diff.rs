use crate::types::{AggregatedStats, DiffResult, Snapshot, TestDiff};
use std::fs;
use std::path::Path;

pub fn compute_diff(
    baseline_path: &Path,
    target_path: &Path,
    out_path: Option<&Path>,
) -> Result<DiffResult, Box<dyn std::error::Error>> {
    let baseline_data = fs::read_to_string(baseline_path)?;
    let baseline: Snapshot = serde_json::from_str(&baseline_data)?;

    let target_data = fs::read_to_string(target_path)?;
    let target: Snapshot = serde_json::from_str(&target_data)?;

    let mut test_diffs = Vec::new();
    let mut regressions = 0;
    let improvements = 0; // Keeping simple for now
    let mut unchanged = 0;

    for (b_res, t_res) in baseline.results.iter().zip(target.results.iter()) {
        let output_changed = b_res.output_text != t_res.output_text;
        
        let tool_calls_changed = b_res.tool_calls_made.len() != t_res.tool_calls_made.len() ||
            b_res.tool_calls_made.iter().zip(t_res.tool_calls_made.iter()).any(|(b, t)| b.name != t.name || b.arguments != t.arguments);
            
        let latency_delta = t_res.latency_ms as i64 - b_res.latency_ms as i64;
        let cost_delta = t_res.estimated_cost - b_res.estimated_cost;

        if output_changed || tool_calls_changed {
            regressions += 1; // Simplification
        } else {
            unchanged += 1;
        }

        test_diffs.push(TestDiff {
            input: b_res.input.clone(),
            output_changed,
            tool_calls_changed,
            latency_delta,
            cost_delta,
        });
    }

    let stats = AggregatedStats {
        total_regressions: regressions,
        total_improvements: improvements,
        unchanged,
    };

    let summary = format!("{} regressions, {} improvements, {} unchanged", regressions, improvements, unchanged);

    let diff_result = DiffResult {
        test_diffs,
        stats,
        summary,
    };

    if let Some(path) = out_path {
        let diff_json = serde_json::to_string_pretty(&diff_result)?;
        fs::write(path, diff_json)?;
        println!("Diff saved to {}", path.display());
    }

    Ok(diff_result)
}
