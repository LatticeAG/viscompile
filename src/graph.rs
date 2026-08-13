//! Graphviz/DOT rendering for behavioral diff reports.
//!
//! Node identifiers are generated from the report position rather than case
//! identifiers. This makes the output valid even when an input case ID contains
//! characters with special meaning in the DOT language.

use crate::types::{CaseStatus, DiffReport};
use std::fmt::Write;

/// Render a deterministic Graphviz/DOT graph of the cases in a diff report.
///
/// Each case is connected to a single report node. The edge and node colours
/// make regressions and improvements apparent in Graphviz viewers while labels
/// retain the status and changed fields for machine-generated reports.
pub fn format_diff_dot(report: &DiffReport) -> String {
    let summary = &report.summary;
    let initial_capacity = 1_024usize
        .saturating_add(report.cases.len().saturating_mul(192))
        .min(1_048_576);
    let mut output = String::with_capacity(initial_capacity);

    output.push_str("digraph viscompile {\n");
    output.push_str("  graph [rankdir=LR, bgcolor=\"transparent\", fontname=\"Arial\", labelloc=t, label=\"VisCompile Behavioral Diff\"];\n");
    output.push_str(
        "  node [fontname=\"Arial\", shape=box, style=\"rounded,filled\", margin=\"0.16,0.10\"];\n",
    );
    output.push_str("  edge [fontname=\"Arial\"];\n");

    let report_label = format!(
        "Diff summary\n{} regressions | {} improvements | {} changed | {} added | {} removed | {} unchanged{}",
        summary.regressions,
        summary.improvements,
        summary.changed,
        summary.added,
        summary.removed,
        summary.unchanged,
        if report.cases.is_empty() {
            "\nNo cases were compared."
        } else {
            ""
        }
    );
    push_node(&mut output, "report", &report_label, "#57606a", "#f6f8fa");
    push_node(
        &mut output,
        "baseline",
        "Baseline transcript",
        "#57606a",
        "#f6f8fa",
    );
    push_node(
        &mut output,
        "target",
        "Target transcript",
        "#57606a",
        "#f6f8fa",
    );
    output.push_str("  baseline -> report [color=\"#57606a\", label=\"baseline\"];\n");
    output.push_str("  report -> target [color=\"#57606a\", label=\"target\"];\n");

    for (index, case) in report.cases.iter().enumerate() {
        let node_id = format!("case_{index}");
        let fields = if case.changed_fields.is_empty() {
            "none".to_string()
        } else {
            case.changed_fields.join(", ")
        };
        let gate = if case.regression {
            "regression"
        } else {
            "informational"
        };
        let label = format!(
            "{}\n{} ({})\nchanged fields: {}",
            case.id,
            status_name(&case.status),
            gate,
            fields
        );
        let (color, fill_color) = status_colors(&case.status);

        push_node(&mut output, &node_id, &label, color, fill_color);
        output.push_str("  report -> ");
        output.push_str(&node_id);
        output.push_str(" [color=\"");
        output.push_str(color);
        output.push_str("\", label=\"");
        push_dot_escaped(&mut output, status_name(&case.status));
        output.push_str("\"];\n");
    }

    output.push_str("}\n");
    output
}

fn push_node(output: &mut String, node_id: &str, label: &str, color: &str, fill_color: &str) {
    output.push_str("  ");
    output.push_str(node_id);
    output.push_str(" [color=\"");
    output.push_str(color);
    output.push_str("\", fillcolor=\"");
    output.push_str(fill_color);
    output.push_str("\", label=\"");
    push_dot_escaped(output, label);
    output.push_str("\"];\n");
}

fn push_dot_escaped(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                // DOT has no portable Unicode escape syntax. Retain a visible,
                // syntax-safe representation instead of embedding a control byte.
                write!(output, "\\\\u{{{:04X}}}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            _ => output.push(character),
        }
    }
}

fn status_colors(status: &CaseStatus) -> (&'static str, &'static str) {
    match status {
        CaseStatus::Unchanged => ("#57606a", "#f6f8fa"),
        CaseStatus::Added => ("#1a7f37", "#dafbe1"),
        CaseStatus::Removed => ("#cf222e", "#ffebe9"),
        CaseStatus::Changed => ("#cf222e", "#ffebe9"),
        CaseStatus::Improved => ("#1a7f37", "#dafbe1"),
    }
}

fn status_name(status: &CaseStatus) -> &'static str {
    match status {
        CaseStatus::Unchanged => "unchanged",
        CaseStatus::Added => "added",
        CaseStatus::Removed => "removed",
        CaseStatus::Changed => "changed",
        CaseStatus::Improved => "improved",
    }
}
