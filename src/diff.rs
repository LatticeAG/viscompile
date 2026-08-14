use crate::canonical::load_snapshot;
use crate::types::{
    CanonicalCase, CanonicalSnapshot, CaseDiff, CaseStatus, DiffReport, DiffSummary, Result,
    DIFF_KIND, SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn diff_paths(baseline_path: &Path, target_path: &Path) -> Result<DiffReport> {
    let baseline = load_snapshot(baseline_path)?;
    let target = load_snapshot(target_path)?;
    Ok(diff_snapshots(&baseline, &target))
}

pub fn diff_snapshots(baseline: &CanonicalSnapshot, target: &CanonicalSnapshot) -> DiffReport {
    let baseline_cases = cases_by_id(baseline);
    let target_cases = cases_by_id(target);
    let identifiers: BTreeSet<&str> = baseline_cases
        .keys()
        .chain(target_cases.keys())
        .copied()
        .collect();

    let mut summary = DiffSummary {
        unchanged: 0,
        added: 0,
        removed: 0,
        changed: 0,
        regressions: 0,
        improvements: 0,
    };
    let mut cases = Vec::with_capacity(identifiers.len());

    for id in identifiers {
        let case_diff = match (baseline_cases.get(id), target_cases.get(id)) {
            (Some(_), None) => {
                summary.removed += 1;
                summary.regressions += 1;
                CaseDiff {
                    id: id.to_string(),
                    status: CaseStatus::Removed,
                    regression: true,
                    changed_fields: vec!["missing_target".to_string()],
                }
            }
            (None, Some(_)) => {
                summary.added += 1;
                CaseDiff {
                    id: id.to_string(),
                    status: CaseStatus::Added,
                    regression: false,
                    changed_fields: vec!["missing_baseline".to_string()],
                }
            }
            (Some(baseline_case), Some(target_case)) => {
                let changed_fields = changed_fields(baseline_case, target_case);
                if changed_fields.is_empty() {
                    summary.unchanged += 1;
                    CaseDiff {
                        id: id.to_string(),
                        status: CaseStatus::Unchanged,
                        regression: false,
                        changed_fields,
                    }
                } else if baseline_case.error.is_some() && target_case.output.is_present() {
                    summary.improvements += 1;
                    CaseDiff {
                        id: id.to_string(),
                        status: CaseStatus::Improved,
                        regression: false,
                        changed_fields,
                    }
                } else {
                    summary.changed += 1;
                    summary.regressions += 1;
                    CaseDiff {
                        id: id.to_string(),
                        status: CaseStatus::Changed,
                        regression: true,
                        changed_fields,
                    }
                }
            }
            (None, None) => unreachable!("case identifier is sourced from one of the snapshots"),
        };
        cases.push(case_diff);
    }

    DiffReport {
        kind: DIFF_KIND.to_string(),
        schema_version: SCHEMA_VERSION,
        summary,
        cases,
    }
}

fn cases_by_id(snapshot: &CanonicalSnapshot) -> BTreeMap<&str, &CanonicalCase> {
    snapshot
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect()
}

fn changed_fields(baseline: &CanonicalCase, target: &CanonicalCase) -> Vec<String> {
    let mut fields = Vec::new();
    if baseline.input != target.input {
        fields.push("input".to_string());
    }
    if baseline.tool_calls != target.tool_calls {
        fields.push("tool_calls".to_string());
    }
    if baseline.output != target.output {
        fields.push("output".to_string());
    }
    if baseline.error != target.error {
        fields.push("error".to_string());
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CanonicalCase, CanonicalOutput, CanonicalToolCall, SNAPSHOT_KIND};
    use serde_json::json;

    fn snapshot(cases: Vec<CanonicalCase>) -> CanonicalSnapshot {
        CanonicalSnapshot {
            kind: SNAPSHOT_KIND.to_string(),
            schema_version: SCHEMA_VERSION,
            cases,
        }
    }

    fn output_case(id: &str, output: serde_json::Value) -> CanonicalCase {
        CanonicalCase {
            id: id.to_string(),
            input: json!("input"),
            tool_calls: Vec::new(),
            output: CanonicalOutput::Value(output),
            error: None,
        }
    }

    fn error_case(id: &str, message: &str) -> CanonicalCase {
        CanonicalCase {
            id: id.to_string(),
            input: json!("input"),
            tool_calls: vec![CanonicalToolCall {
                name: "lookup".to_string(),
                arguments: json!({}),
            }],
            output: CanonicalOutput::Absent,
            error: Some(message.to_string()),
        }
    }

    #[test]
    fn diff_snapshots_marks_error_resolution_as_improvement() {
        let baseline = snapshot(vec![error_case("lookup", "upstream unavailable")]);
        let target = snapshot(vec![output_case("lookup", json!("Record found."))]);
        let report = diff_snapshots(&baseline, &target);
        assert_eq!(report.summary.improvements, 1);
        assert_eq!(report.summary.regressions, 0);
        assert_eq!(report.cases[0].status, CaseStatus::Improved);
    }

    #[test]
    fn diff_snapshots_marks_changed_output_as_regression() {
        let baseline = snapshot(vec![output_case("capital", json!("Paris."))]);
        let target = snapshot(vec![output_case("capital", json!("Lyon."))]);
        let report = diff_snapshots(&baseline, &target);
        assert_eq!(report.summary.changed, 1);
        assert_eq!(report.summary.regressions, 1);
        assert_eq!(report.cases[0].status, CaseStatus::Changed);
        assert_eq!(report.cases[0].changed_fields, vec!["output".to_string()]);
    }
}
