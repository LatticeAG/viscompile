use crate::canonical::snapshot_digest;
use crate::diff::diff_snapshots;
use crate::types::{
    AlignmentRelation, CanonicalCase, CanonicalSnapshot, CanonicalToolCall, CaseComparison,
    CaseStatus, ChangeOperation, ChangeRecord, ComparisonReport, Result, ToolCallAlignment,
    ValueSlot, WhyCode, COMPARISON_KIND, COMPARISON_PROFILE, SCHEMA_VERSION,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub fn compare_snapshots(
    baseline: &CanonicalSnapshot,
    target: &CanonicalSnapshot,
) -> Result<ComparisonReport> {
    let structural = diff_snapshots(baseline, target);
    let baseline_digest = snapshot_digest(baseline)?;
    let target_digest = snapshot_digest(target)?;
    let baseline_cases = cases_by_id(baseline);
    let target_cases = cases_by_id(target);

    let mut cases = Vec::with_capacity(structural.cases.len());
    for case_diff in &structural.cases {
        let baseline_case = baseline_cases.get(case_diff.id.as_str()).copied();
        let target_case = target_cases.get(case_diff.id.as_str()).copied();
        let (changes, tool_call_alignment) =
            case_changes_and_alignment(case_diff.status, baseline_case, target_case);
        let why_codes = why_codes_for(
            &case_diff.status,
            &case_diff.changed_fields,
            baseline_case,
            target_case,
            &tool_call_alignment,
        );
        cases.push(CaseComparison {
            id: case_diff.id.clone(),
            status: case_diff.status,
            regression: case_diff.regression,
            changed_fields: case_diff.changed_fields.clone(),
            why_codes,
            changes,
            tool_call_alignment,
        });
    }

    Ok(ComparisonReport {
        kind: COMPARISON_KIND.to_string(),
        schema_version: SCHEMA_VERSION,
        profile: COMPARISON_PROFILE.to_string(),
        baseline_digest,
        target_digest,
        summary: structural.summary,
        cases,
    })
}

fn cases_by_id(snapshot: &CanonicalSnapshot) -> std::collections::BTreeMap<&str, &CanonicalCase> {
    snapshot
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect()
}

fn case_changes_and_alignment(
    status: CaseStatus,
    baseline: Option<&CanonicalCase>,
    target: Option<&CanonicalCase>,
) -> (Vec<ChangeRecord>, Vec<ToolCallAlignment>) {
    match status {
        CaseStatus::Unchanged => {
            let calls = baseline
                .map(|case| case.tool_calls.as_slice())
                .unwrap_or(&[]);
            let alignment = calls
                .iter()
                .enumerate()
                .map(|(index, _)| ToolCallAlignment {
                    baseline_index: Some(index as u64),
                    target_index: Some(index as u64),
                    relation: AlignmentRelation::Equal,
                })
                .collect();
            (Vec::new(), alignment)
        }
        CaseStatus::Added => {
            let case = target.expect("added cases have a target");
            let mut changes = vec![
                change_record(
                    "/input",
                    ChangeOperation::Add,
                    ValueSlot::absent(),
                    ValueSlot::present(case.input.clone()),
                ),
                change_record(
                    "/tool_calls",
                    ChangeOperation::Add,
                    ValueSlot::absent(),
                    ValueSlot::present(tool_calls_value(&case.tool_calls)),
                ),
            ];
            changes.extend(terminal_changes(None, Some(case)));
            sort_changes(&mut changes);
            let alignment = case
                .tool_calls
                .iter()
                .enumerate()
                .map(|(index, _)| ToolCallAlignment {
                    baseline_index: None,
                    target_index: Some(index as u64),
                    relation: AlignmentRelation::Insert,
                })
                .collect();
            (changes, alignment)
        }
        CaseStatus::Removed => {
            let case = baseline.expect("removed cases have a baseline");
            let mut changes = vec![
                change_record(
                    "/input",
                    ChangeOperation::Remove,
                    ValueSlot::present(case.input.clone()),
                    ValueSlot::absent(),
                ),
                change_record(
                    "/tool_calls",
                    ChangeOperation::Remove,
                    ValueSlot::present(tool_calls_value(&case.tool_calls)),
                    ValueSlot::absent(),
                ),
            ];
            changes.extend(terminal_changes(Some(case), None));
            sort_changes(&mut changes);
            let alignment = case
                .tool_calls
                .iter()
                .enumerate()
                .map(|(index, _)| ToolCallAlignment {
                    baseline_index: Some(index as u64),
                    target_index: None,
                    relation: AlignmentRelation::Remove,
                })
                .collect();
            (changes, alignment)
        }
        CaseStatus::Changed | CaseStatus::Improved => {
            let baseline = baseline.expect("changed cases have a baseline");
            let target = target.expect("changed cases have a target");
            let mut changes = diff_values(&baseline.input, &target.input, "/input");
            changes.extend(diff_output_field(&baseline.output, &target.output));
            changes.extend(diff_error_field(
                baseline.error.as_deref(),
                target.error.as_deref(),
            ));
            let alignment = align_tool_calls(&baseline.tool_calls, &target.tool_calls);
            changes.extend(tool_call_changes(
                &baseline.tool_calls,
                &target.tool_calls,
                &alignment,
            ));
            sort_changes(&mut changes);
            (changes, alignment)
        }
    }
}

fn terminal_changes(
    baseline: Option<&CanonicalCase>,
    target: Option<&CanonicalCase>,
) -> Vec<ChangeRecord> {
    match (baseline, target) {
        (None, Some(case)) => {
            if let Some(value) = output_json(&case.output) {
                vec![change_record(
                    "/output",
                    ChangeOperation::Add,
                    ValueSlot::absent(),
                    ValueSlot::present(value),
                )]
            } else if let Some(message) = &case.error {
                vec![change_record(
                    "/error",
                    ChangeOperation::Add,
                    ValueSlot::absent(),
                    ValueSlot::present(Value::String(message.clone())),
                )]
            } else {
                Vec::new()
            }
        }
        (Some(case), None) => {
            if let Some(value) = output_json(&case.output) {
                vec![change_record(
                    "/output",
                    ChangeOperation::Remove,
                    ValueSlot::present(value),
                    ValueSlot::absent(),
                )]
            } else if let Some(message) = &case.error {
                vec![change_record(
                    "/error",
                    ChangeOperation::Remove,
                    ValueSlot::present(Value::String(message.clone())),
                    ValueSlot::absent(),
                )]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn output_json(output: &crate::types::CanonicalOutput) -> Option<Value> {
    match output {
        crate::types::CanonicalOutput::Absent => None,
        crate::types::CanonicalOutput::Value(value) => Some(value.clone()),
    }
}

fn diff_output_field(
    baseline: &crate::types::CanonicalOutput,
    target: &crate::types::CanonicalOutput,
) -> Vec<ChangeRecord> {
    match (output_json(baseline), output_json(target)) {
        (None, None) => Vec::new(),
        (None, Some(after)) => vec![change_record(
            "/output",
            ChangeOperation::Add,
            ValueSlot::absent(),
            ValueSlot::present(after),
        )],
        (Some(before), None) => vec![change_record(
            "/output",
            ChangeOperation::Remove,
            ValueSlot::present(before),
            ValueSlot::absent(),
        )],
        (Some(before), Some(after)) => diff_values(&before, &after, "/output"),
    }
}

fn diff_error_field(baseline: Option<&str>, target: Option<&str>) -> Vec<ChangeRecord> {
    match (baseline, target) {
        (None, None) => Vec::new(),
        (None, Some(after)) => vec![change_record(
            "/error",
            ChangeOperation::Add,
            ValueSlot::absent(),
            ValueSlot::present(Value::String(after.to_string())),
        )],
        (Some(before), None) => vec![change_record(
            "/error",
            ChangeOperation::Remove,
            ValueSlot::present(Value::String(before.to_string())),
            ValueSlot::absent(),
        )],
        (Some(before), Some(after)) if before == after => Vec::new(),
        (Some(before), Some(after)) => diff_values(
            &Value::String(before.to_string()),
            &Value::String(after.to_string()),
            "/error",
        ),
    }
}

fn tool_calls_value(calls: &[CanonicalToolCall]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                json!({
                    "name": call.name,
                    "arguments": call.arguments,
                })
            })
            .collect(),
    )
}

fn tool_call_object(call: &CanonicalToolCall) -> Value {
    json!({
        "name": call.name,
        "arguments": call.arguments,
    })
}

fn tool_call_changes(
    baseline: &[CanonicalToolCall],
    target: &[CanonicalToolCall],
    alignment: &[ToolCallAlignment],
) -> Vec<ChangeRecord> {
    let mut changes = Vec::new();
    for row in alignment {
        match row.relation {
            AlignmentRelation::Equal => {}
            AlignmentRelation::Replace => {
                let baseline_index = row.baseline_index.expect("replace has baseline") as usize;
                let target_index = row.target_index.expect("replace has target") as usize;
                let pointer = format!("/tool_calls/{baseline_index}/arguments");
                changes.extend(diff_values(
                    &baseline[baseline_index].arguments,
                    &target[target_index].arguments,
                    &pointer,
                ));
            }
            AlignmentRelation::Remove => {
                let index = row.baseline_index.expect("remove has baseline") as usize;
                changes.push(change_record(
                    &format!("/tool_calls/{index}"),
                    ChangeOperation::Remove,
                    ValueSlot::present(tool_call_object(&baseline[index])),
                    ValueSlot::absent(),
                ));
            }
            AlignmentRelation::Insert => {
                let index = row.target_index.expect("insert has target") as usize;
                changes.push(change_record(
                    &format!("/tool_calls/{index}"),
                    ChangeOperation::Add,
                    ValueSlot::absent(),
                    ValueSlot::present(tool_call_object(&target[index])),
                ));
            }
        }
    }
    changes
}

pub fn align_tool_calls(
    baseline: &[CanonicalToolCall],
    target: &[CanonicalToolCall],
) -> Vec<ToolCallAlignment> {
    let n = baseline.len();
    let m = target.len();
    let mut dp = vec![vec![0_usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if baseline[i].name == target[j].name {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut alignments = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < n || j < m {
        if i < n && j < m && baseline[i].name == target[j].name {
            let relation = if baseline[i].arguments == target[j].arguments {
                AlignmentRelation::Equal
            } else {
                AlignmentRelation::Replace
            };
            alignments.push(ToolCallAlignment {
                baseline_index: Some(i as u64),
                target_index: Some(j as u64),
                relation,
            });
            i += 1;
            j += 1;
        } else if i < n && (j == m || dp[i + 1][j] >= dp[i][j + 1]) {
            alignments.push(ToolCallAlignment {
                baseline_index: Some(i as u64),
                target_index: None,
                relation: AlignmentRelation::Remove,
            });
            i += 1;
        } else {
            alignments.push(ToolCallAlignment {
                baseline_index: None,
                target_index: Some(j as u64),
                relation: AlignmentRelation::Insert,
            });
            j += 1;
        }
    }
    alignments
}

pub fn diff_values(baseline: &Value, target: &Value, pointer: &str) -> Vec<ChangeRecord> {
    if baseline == target {
        return Vec::new();
    }
    if json_type(baseline) != json_type(target) {
        return vec![change_record(
            pointer,
            ChangeOperation::TypeChange,
            ValueSlot::present(baseline.clone()),
            ValueSlot::present(target.clone()),
        )];
    }
    match (baseline, target) {
        (Value::Object(left), Value::Object(right)) => diff_objects(left, right, pointer),
        (Value::Array(left), Value::Array(right)) => diff_arrays(left, right, pointer),
        _ => vec![change_record(
            pointer,
            ChangeOperation::Replace,
            ValueSlot::present(baseline.clone()),
            ValueSlot::present(target.clone()),
        )],
    }
}

fn diff_objects(
    left: &Map<String, Value>,
    right: &Map<String, Value>,
    pointer: &str,
) -> Vec<ChangeRecord> {
    let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    let mut changes = Vec::new();
    for key in keys {
        let child = append_pointer(pointer, key);
        match (left.get(key), right.get(key)) {
            (Some(_), None) => changes.push(change_record(
                &child,
                ChangeOperation::Remove,
                ValueSlot::present(left[key].clone()),
                ValueSlot::absent(),
            )),
            (None, Some(after)) => changes.push(change_record(
                &child,
                ChangeOperation::Add,
                ValueSlot::absent(),
                ValueSlot::present(after.clone()),
            )),
            (Some(before), Some(after)) => changes.extend(diff_values(before, after, &child)),
            (None, None) => {}
        }
    }
    changes
}

fn diff_arrays(left: &[Value], right: &[Value], pointer: &str) -> Vec<ChangeRecord> {
    let mut changes = Vec::new();
    let shared = left.len().min(right.len());
    for index in 0..shared {
        let child = format!("{pointer}/{index}");
        changes.extend(diff_values(&left[index], &right[index], &child));
    }
    for (index, value) in left.iter().enumerate().skip(shared) {
        changes.push(change_record(
            &format!("{pointer}/{index}"),
            ChangeOperation::Remove,
            ValueSlot::present(value.clone()),
            ValueSlot::absent(),
        ));
    }
    for (index, value) in right.iter().enumerate().skip(shared) {
        changes.push(change_record(
            &format!("{pointer}/{index}"),
            ChangeOperation::Add,
            ValueSlot::absent(),
            ValueSlot::present(value.clone()),
        ));
    }
    changes
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn append_pointer(base: &str, segment: &str) -> String {
    format!("{base}/{}", escape_pointer_segment(segment))
}

fn escape_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn change_record(
    path: &str,
    operation: ChangeOperation,
    before: ValueSlot,
    after: ValueSlot,
) -> ChangeRecord {
    ChangeRecord {
        path: path.to_string(),
        operation,
        before,
        after,
    }
}

fn sort_changes(changes: &mut [ChangeRecord]) {
    changes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.operation.rank().cmp(&right.operation.rank()))
    });
}

fn tool_call_multiset(calls: &[CanonicalToolCall]) -> Vec<(String, String)> {
    let mut items: Vec<(String, String)> = calls
        .iter()
        .map(|call| {
            (
                call.name.clone(),
                serde_json::to_string(&call.arguments).unwrap_or_default(),
            )
        })
        .collect();
    items.sort();
    items
}

fn is_reordered(baseline: &[CanonicalToolCall], target: &[CanonicalToolCall]) -> bool {
    baseline != target && tool_call_multiset(baseline) == tool_call_multiset(target)
}

pub fn why_codes_for(
    status: &CaseStatus,
    changed_fields: &[String],
    baseline: Option<&CanonicalCase>,
    target: Option<&CanonicalCase>,
    alignment: &[ToolCallAlignment],
) -> Vec<WhyCode> {
    let mut codes = Vec::new();
    let push_code = |codes: &mut Vec<WhyCode>, code: WhyCode| {
        if !codes.contains(&code) {
            codes.push(code);
        }
    };

    if *status == CaseStatus::Added {
        push_code(&mut codes, WhyCode::CaseAdded);
    }
    if *status == CaseStatus::Removed {
        push_code(&mut codes, WhyCode::CaseRemoved);
    }
    if *status == CaseStatus::Improved {
        push_code(&mut codes, WhyCode::ErrorResolved);
    }
    if baseline.is_some_and(|case| case.output.is_present())
        && target.is_some_and(|case| case.error.is_some())
    {
        push_code(&mut codes, WhyCode::ErrorRaised);
    }
    if changed_fields.iter().any(|field| field == "input") {
        push_code(&mut codes, WhyCode::InputChanged);
    }

    let reordered = match (baseline, target) {
        (Some(left), Some(right)) => is_reordered(&left.tool_calls, &right.tool_calls),
        _ => false,
    };
    if reordered {
        push_code(&mut codes, WhyCode::ToolCallReordered);
    } else {
        if alignment
            .iter()
            .any(|row| row.relation == AlignmentRelation::Remove)
        {
            push_code(&mut codes, WhyCode::ToolCallRemoved);
        }
        if alignment
            .iter()
            .any(|row| row.relation == AlignmentRelation::Insert)
        {
            push_code(&mut codes, WhyCode::ToolCallAdded);
        }
    }
    if alignment
        .iter()
        .any(|row| row.relation == AlignmentRelation::Replace)
    {
        push_code(&mut codes, WhyCode::ToolCallArgumentsChanged);
    }
    if changed_fields.iter().any(|field| field == "output") {
        push_code(&mut codes, WhyCode::OutputChanged);
    }
    if changed_fields.iter().any(|field| field == "error")
        && !codes.contains(&WhyCode::ErrorResolved)
        && !codes.contains(&WhyCode::ErrorRaised)
    {
        push_code(&mut codes, WhyCode::ErrorChanged);
    }

    codes.sort();
    codes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CanonicalOutput, CanonicalToolCall};
    use serde_json::json;

    fn call(name: &str, arguments: Value) -> CanonicalToolCall {
        CanonicalToolCall {
            name: name.to_string(),
            arguments,
        }
    }

    fn case_with_calls(id: &str, calls: Vec<CanonicalToolCall>) -> CanonicalCase {
        CanonicalCase {
            id: id.to_string(),
            input: json!("in"),
            tool_calls: calls,
            output: CanonicalOutput::Value(json!("out")),
            error: None,
        }
    }

    fn snapshot(cases: Vec<CanonicalCase>) -> CanonicalSnapshot {
        CanonicalSnapshot {
            kind: crate::types::SNAPSHOT_KIND.to_string(),
            schema_version: SCHEMA_VERSION,
            cases,
        }
    }

    #[test]
    fn align_tool_calls_inserts_in_the_middle() {
        let baseline = vec![call("search", json!({})), call("write", json!({}))];
        let target = vec![
            call("search", json!({})),
            call("read", json!({})),
            call("write", json!({})),
        ];
        let alignment = align_tool_calls(&baseline, &target);
        assert_eq!(alignment.len(), 3);
        assert_eq!(alignment[0].relation, AlignmentRelation::Equal);
        assert_eq!(alignment[0].baseline_index, Some(0));
        assert_eq!(alignment[0].target_index, Some(0));
        assert_eq!(alignment[1].relation, AlignmentRelation::Insert);
        assert_eq!(alignment[1].target_index, Some(1));
        assert_eq!(alignment[2].relation, AlignmentRelation::Equal);
        assert_eq!(alignment[2].baseline_index, Some(1));
        assert_eq!(alignment[2].target_index, Some(2));
    }

    #[test]
    fn align_tool_calls_argument_replace_same_name() {
        let baseline = vec![call("search", json!({"q": "a"}))];
        let target = vec![call("search", json!({"q": "b"}))];
        let alignment = align_tool_calls(&baseline, &target);
        assert_eq!(alignment.len(), 1);
        assert_eq!(alignment[0].relation, AlignmentRelation::Replace);
    }

    #[test]
    fn diff_values_escapes_tilde_and_slash_in_keys() {
        let baseline = json!({"a/b": 1, "c~d": 2});
        let target = json!({"a/b": 3, "c~d": 4});
        let changes = diff_values(&baseline, &target, "/input");
        let paths: Vec<&str> = changes.iter().map(|change| change.path.as_str()).collect();
        assert!(paths.contains(&"/input/a~1b"));
        assert!(paths.contains(&"/input/c~0d"));
    }

    #[test]
    fn why_codes_error_resolved_skips_error_changed() {
        let baseline = CanonicalCase {
            id: "lookup".to_string(),
            input: json!("in"),
            tool_calls: Vec::new(),
            output: CanonicalOutput::Absent,
            error: Some("boom".to_string()),
        };
        let target = CanonicalCase {
            id: "lookup".to_string(),
            input: json!("in"),
            tool_calls: Vec::new(),
            output: CanonicalOutput::Value(json!("ok")),
            error: None,
        };
        let codes = why_codes_for(
            &CaseStatus::Improved,
            &["error".to_string(), "output".to_string()],
            Some(&baseline),
            Some(&target),
            &[],
        );
        assert!(codes.contains(&WhyCode::ErrorResolved));
        assert!(!codes.contains(&WhyCode::ErrorChanged));
        assert!(codes.contains(&WhyCode::OutputChanged));
    }

    #[test]
    fn compare_snapshots_projects_v1_summary() {
        let baseline = snapshot(vec![
            case_with_calls("kept", Vec::new()),
            case_with_calls("removed", Vec::new()),
        ]);
        let target = snapshot(vec![
            case_with_calls("added", Vec::new()),
            case_with_calls("kept", Vec::new()),
        ]);
        let comparison = compare_snapshots(&baseline, &target).expect("compare");
        let structural = diff_snapshots(&baseline, &target);
        assert_eq!(comparison.summary, structural.summary);
        for (left, right) in comparison.cases.iter().zip(structural.cases.iter()) {
            assert_eq!(left.id, right.id);
            assert_eq!(left.status, right.status);
            assert_eq!(left.regression, right.regression);
            assert_eq!(left.changed_fields, right.changed_fields);
        }
    }

    #[test]
    fn reorder_suppresses_removed_and_added_why_codes() {
        let baseline = snapshot(vec![case_with_calls(
            "one",
            vec![call("search", json!({})), call("write", json!({}))],
        )]);
        let target = snapshot(vec![case_with_calls(
            "one",
            vec![call("write", json!({})), call("search", json!({}))],
        )]);
        let comparison = compare_snapshots(&baseline, &target).expect("compare");
        let codes = &comparison.cases[0].why_codes;
        assert!(codes.contains(&WhyCode::ToolCallReordered));
        assert!(!codes.contains(&WhyCode::ToolCallRemoved));
        assert!(!codes.contains(&WhyCode::ToolCallAdded));
    }
}
