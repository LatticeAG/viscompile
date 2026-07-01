use crate::types::DiffResult;

pub fn format_diff_json(diff: &DiffResult) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(diff)
}

pub fn format_diff_text(diff: &DiffResult) -> String {
    let mut out = String::new();
    out.push_str("Behavioral Diff Summary\n");
    out.push_str("=======================\n\n");
    out.push_str(&diff.summary);
    out.push_str("\n\nDetails:\n");
    for td in &diff.test_diffs {
        out.push_str(&format!(
            "- Input: {}\n  Output Changed: {}\n  Tools Changed: {}\n  Latency Delta: {}ms\n",
            td.input, td.output_changed, td.tool_calls_changed, td.latency_delta
        ));
    }
    out
}
