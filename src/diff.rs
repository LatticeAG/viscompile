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
