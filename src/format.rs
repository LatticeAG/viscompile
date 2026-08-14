use crate::canonical::canonical_json;
use crate::types::{escape_untrusted_text, CaseStatus, DiffReport, Result};
use std::fmt::Write as _;

pub fn format_diff_json(report: &DiffReport) -> Result<String> {
    canonical_json(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTextKind {
    Heading,
    SummaryOk,
    SummaryRegressed,
    Body,
    StatusUnchanged,
    StatusAdded,
    StatusRemoved,
    StatusChanged,
    StatusImproved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffTextLine {
    pub kind: DiffTextKind,
    pub text: String,
}

pub fn diff_text_lines(report: &DiffReport) -> Vec<DiffTextLine> {
    let summary = &report.summary;
    let mut lines = Vec::new();
    lines.push(DiffTextLine {
        kind: DiffTextKind::Heading,
        text: "VisCompile Behavioral Diff\n==========================\n\n".to_string(),
    });
    lines.push(DiffTextLine {
        kind: if summary.regressions > 0 {
            DiffTextKind::SummaryRegressed
        } else {
            DiffTextKind::SummaryOk
        },
        text: format!(
            "{} regressions, {} improvements, {} changed, {} added, {} removed, {} unchanged\n",
            summary.regressions,
            summary.improvements,
            summary.changed,
            summary.added,
            summary.removed,
            summary.unchanged
        ),
    });
    lines.push(DiffTextLine {
        kind: DiffTextKind::Body,
        text: "\nCases:\n".to_string(),
    });

    for case in &report.cases {
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
        lines.push(DiffTextLine {
            kind: DiffTextKind::Body,
            text: "- [".to_string(),
        });
        lines.push(DiffTextLine {
            kind: status_kind(&case.status),
            text: status_name(&case.status).to_string(),
        });
        lines.push(DiffTextLine {
            kind: DiffTextKind::Body,
            text: format!("] {}: {fields} ({gate})\n", escape_text(&case.id)),
        });
    }
    lines
}

pub fn format_diff_text(report: &DiffReport) -> String {
    diff_text_lines(report)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

fn status_kind(status: &CaseStatus) -> DiffTextKind {
    match status {
        CaseStatus::Unchanged => DiffTextKind::StatusUnchanged,
        CaseStatus::Added => DiffTextKind::StatusAdded,
        CaseStatus::Removed => DiffTextKind::StatusRemoved,
        CaseStatus::Changed => DiffTextKind::StatusChanged,
        CaseStatus::Improved => DiffTextKind::StatusImproved,
    }
}

/// Render untrusted text safely in a terminal-oriented report.
///
/// Case IDs originate in input transcripts, so control bytes must be visible rather than allowed
/// to alter terminal state or inject additional report lines.
pub fn escape_text(value: &str) -> String {
    escape_untrusted_text(value)
}

/// Render a self-contained HTML representation of a behavioral diff.
///
/// The report is deliberately rendered without JavaScript or external assets so
/// it can be saved and inspected offline. All report-derived text is escaped
/// before it is inserted into the document.
pub fn format_diff_html(report: &DiffReport) -> String {
    let summary = &report.summary;
    let initial_capacity = 2_048usize
        .saturating_add(report.cases.len().saturating_mul(256))
        .min(1_048_576);
    let mut output = String::with_capacity(initial_capacity);

    output.push_str("<!doctype html>\n");
    output.push_str("<html lang=\"en\">\n<head>\n");
    output.push_str("  <meta charset=\"utf-8\">\n");
    output.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    output.push_str("  <title>VisCompile Behavioral Diff</title>\n");
    output.push_str("  <style>\n");
    output.push_str("    :root { color-scheme: light dark; font-family: system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif; }\n");
    output.push_str(
        "    body { margin: 2rem auto; max-width: 72rem; padding: 0 1rem; line-height: 1.5; }\n",
    );
    output.push_str("    h1 { margin-bottom: .25rem; }\n");
    output.push_str("    table { border-collapse: collapse; margin: 1.5rem 0; width: 100%; }\n");
    output.push_str("    th, td { border: 1px solid #8b949e; padding: .65rem .75rem; text-align: left; vertical-align: top; }\n");
    output.push_str("    th { font-weight: 650; }\n");
    output.push_str("    .summary { display: flex; flex-wrap: wrap; gap: .75rem; list-style: none; margin: 1rem 0 2rem; padding: 0; }\n");
    output.push_str("    .summary li { border: 1px solid #8b949e; border-radius: .35rem; padding: .5rem .75rem; }\n");
    output.push_str("    .status { font-weight: 700; text-transform: capitalize; }\n");
    output.push_str("    .status-unchanged { color: #57606a; }\n");
    output.push_str("    .status-added, .status-improved { color: #1a7f37; }\n");
    output.push_str("    .status-removed, .status-changed { color: #cf222e; }\n");
    output.push_str("    .regression { color: #cf222e; font-weight: 700; }\n");
    output.push_str("    .informational { color: #57606a; }\n");
    output.push_str("    .case-id, .fields { overflow-wrap: anywhere; }\n");
    output.push_str("  </style>\n</head>\n<body>\n");
    output.push_str("  <main>\n");
    output.push_str("    <h1>VisCompile Behavioral Diff</h1>\n");
    output.push_str("    <p>Deterministic behavioral comparison of agent transcript cases.</p>\n");
    output.push_str("    <ul class=\"summary\" aria-label=\"Diff summary\">\n");
    push_summary_item(&mut output, "Regressions", summary.regressions);
    push_summary_item(&mut output, "Improvements", summary.improvements);
    push_summary_item(&mut output, "Changed", summary.changed);
    push_summary_item(&mut output, "Added", summary.added);
    push_summary_item(&mut output, "Removed", summary.removed);
    push_summary_item(&mut output, "Unchanged", summary.unchanged);
    output.push_str("    </ul>\n");
    output.push_str("    <h2>Cases</h2>\n");

    if report.cases.is_empty() {
        output.push_str("    <p>No cases were compared.</p>\n");
    } else {
        output.push_str("    <table>\n");
        output.push_str("      <thead><tr><th scope=\"col\">Case</th><th scope=\"col\">Status</th><th scope=\"col\">Changed fields</th><th scope=\"col\">Gate</th></tr></thead>\n");
        output.push_str("      <tbody>\n");

        for case in &report.cases {
            let status = status_name(&case.status);
            let fields = if case.changed_fields.is_empty() {
                "none".to_string()
            } else {
                case.changed_fields.join(", ")
            };
            let gate = if case.regression {
                "Regression"
            } else {
                "Informational"
            };
            let gate_class = if case.regression {
                "regression"
            } else {
                "informational"
            };

            output.push_str("        <tr><th scope=\"row\" class=\"case-id\">");
            push_html_escaped(&mut output, &case.id);
            output.push_str("</th><td><span class=\"status status-");
            output.push_str(status);
            output.push_str("\">");
            push_html_escaped(&mut output, status);
            output.push_str("</span></td><td class=\"fields\">");
            push_html_escaped(&mut output, &fields);
            output.push_str("</td><td><span class=\"");
            output.push_str(gate_class);
            output.push_str("\">");
            output.push_str(gate);
            output.push_str("</span></td></tr>\n");
        }

        output.push_str("      </tbody>\n    </table>\n");
    }

    output.push_str("  </main>\n</body>\n</html>\n");
    output
}

fn push_summary_item(output: &mut String, label: &str, value: u64) {
    output.push_str("      <li><strong>");
    output.push_str(label);
    output.push_str(":</strong> ");
    output.push_str(&value.to_string());
    output.push_str("</li>\n");
}

pub(crate) fn push_html_escaped(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            character if character.is_control() => {
                write!(output, "&#x{:X};", character as u32)
                    .expect("writing to a String cannot fail");
            }
            _ => output.push(character),
        }
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

pub fn format_comparison_text(report: &crate::types::ComparisonReport) -> String {
    let summary = &report.summary;
    let mut output = String::from("VisCompile Behavioral Comparison\n");
    output.push_str("================================\n\n");
    output.push_str(&format!(
        "{} regressions, {} improvements, {} changed, {} added, {} removed, {} unchanged\n",
        summary.regressions,
        summary.improvements,
        summary.changed,
        summary.added,
        summary.removed,
        summary.unchanged
    ));
    output.push_str(&format!("profile {}\n", report.profile));
    output.push_str(&format!("baseline {}\n", report.baseline_digest));
    output.push_str(&format!("target {}\n", report.target_digest));
    output.push_str("\nCases:\n");

    for case in &report.cases {
        let gate = if case.regression {
            "regression"
        } else {
            "informational"
        };
        output.push_str(&format!(
            "- [{}] {} ({gate})\n",
            status_name(&case.status),
            escape_text(&case.id)
        ));
        if case.why_codes.is_empty() {
            output.push_str("  why: none\n");
        } else {
            let codes = case
                .why_codes
                .iter()
                .map(why_code_name)
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("  why: {codes}\n"));
        }
        for change in &case.changes {
            output.push_str(&format!(
                "  {}: {} -> {}\n",
                change.path,
                preview_value(&change.before),
                preview_value(&change.after)
            ));
        }
    }
    output
}

pub fn format_comparison_html(report: &crate::types::ComparisonReport) -> String {
    let summary = &report.summary;
    let mut output = String::from("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    output.push_str("  <meta charset=\"utf-8\">\n");
    output.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    output.push_str("  <title>VisCompile Behavioral Comparison</title>\n");
    output.push_str("  <style>\n");
    output.push_str("    :root { color-scheme: light dark; font-family: system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif; }\n");
    output.push_str(
        "    body { margin: 2rem auto; max-width: 72rem; padding: 0 1rem; line-height: 1.5; }\n",
    );
    output.push_str("    h1 { margin-bottom: .25rem; }\n");
    output.push_str("    table { border-collapse: collapse; margin: 1.5rem 0; width: 100%; }\n");
    output.push_str("    th, td { border: 1px solid #8b949e; padding: .65rem .75rem; text-align: left; vertical-align: top; }\n");
    output.push_str("    th { font-weight: 650; }\n");
    output.push_str("    .summary { display: flex; flex-wrap: wrap; gap: .75rem; list-style: none; margin: 1rem 0 2rem; padding: 0; }\n");
    output.push_str("    .summary li { border: 1px solid #8b949e; border-radius: .35rem; padding: .5rem .75rem; }\n");
    output.push_str("    .status { font-weight: 700; text-transform: capitalize; }\n");
    output.push_str("    .status-unchanged { color: #57606a; }\n");
    output.push_str("    .status-added, .status-improved { color: #1a7f37; }\n");
    output.push_str("    .status-removed, .status-changed { color: #cf222e; }\n");
    output.push_str("    .regression { color: #cf222e; font-weight: 700; }\n");
    output.push_str("    .informational { color: #57606a; }\n");
    output.push_str("    .case-id, .fields { overflow-wrap: anywhere; }\n");
    output.push_str("  </style>\n</head>\n<body>\n  <main>\n");
    output.push_str("    <h1>VisCompile Behavioral Comparison</h1>\n");
    output.push_str("    <p>Deterministic evidence comparison of canonical snapshot cases.</p>\n");
    output.push_str("    <ul class=\"summary\" aria-label=\"Comparison summary\">\n");
    push_summary_item(&mut output, "Regressions", summary.regressions);
    push_summary_item(&mut output, "Improvements", summary.improvements);
    push_summary_item(&mut output, "Changed", summary.changed);
    push_summary_item(&mut output, "Added", summary.added);
    push_summary_item(&mut output, "Removed", summary.removed);
    push_summary_item(&mut output, "Unchanged", summary.unchanged);
    output.push_str("      <li><strong>Profile:</strong> ");
    push_html_escaped(&mut output, &report.profile);
    output.push_str("</li>\n      <li><strong>Baseline:</strong> ");
    push_html_escaped(&mut output, &report.baseline_digest);
    output.push_str("</li>\n      <li><strong>Target:</strong> ");
    push_html_escaped(&mut output, &report.target_digest);
    output.push_str("</li>\n    </ul>\n    <h2>Cases</h2>\n");

    if report.cases.is_empty() {
        output.push_str("    <p>No cases were compared.</p>\n");
    } else {
        output.push_str("    <table>\n");
        output.push_str("      <thead><tr><th scope=\"col\">Case</th><th scope=\"col\">Status</th><th scope=\"col\">Gate</th><th scope=\"col\">Why</th><th scope=\"col\">Changes</th></tr></thead>\n");
        output.push_str("      <tbody>\n");
        for case in &report.cases {
            let status = status_name(&case.status);
            let gate = if case.regression {
                "Regression"
            } else {
                "Informational"
            };
            let gate_class = if case.regression {
                "regression"
            } else {
                "informational"
            };
            let why = if case.why_codes.is_empty() {
                "none".to_string()
            } else {
                case.why_codes
                    .iter()
                    .map(why_code_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            output.push_str("        <tr><th scope=\"row\" class=\"case-id\">");
            push_html_escaped(&mut output, &case.id);
            output.push_str("</th><td><span class=\"status status-");
            output.push_str(status);
            output.push_str("\">");
            push_html_escaped(&mut output, status);
            output.push_str("</span></td><td><span class=\"");
            output.push_str(gate_class);
            output.push_str("\">");
            output.push_str(gate);
            output.push_str("</span></td><td class=\"fields\">");
            push_html_escaped(&mut output, &why);
            output.push_str("</td><td><ul>");
            for change in &case.changes {
                output.push_str("<li>");
                let line = format!(
                    "{}: {} -> {}",
                    change.path,
                    preview_value(&change.before),
                    preview_value(&change.after)
                );
                push_html_escaped(&mut output, &line);
                output.push_str("</li>");
            }
            output.push_str("</ul></td></tr>\n");
        }
        output.push_str("      </tbody>\n    </table>\n");
    }
    output.push_str("  </main>\n</body>\n</html>\n");
    output
}

pub fn preview_value(slot: &crate::types::ValueSlot) -> String {
    match slot.presence {
        crate::types::Presence::Absent => "ABSENT".to_string(),
        crate::types::Presence::Present => {
            let value = slot.value.as_ref().unwrap_or(&serde_json::Value::Null);
            let preview = match value {
                serde_json::Value::Null => "null".to_string(),
                serde_json::Value::String(text) => format!("\"{}\"", escape_text(text)),
                serde_json::Value::Bool(flag) => flag.to_string(),
                serde_json::Value::Number(number) => number.to_string(),
                other => {
                    let compact =
                        serde_json::to_string(other).unwrap_or_else(|_| "null".to_string());
                    escape_text(&compact)
                }
            };
            truncate_preview(preview, value)
        }
    }
}

fn truncate_preview(preview: String, value: &serde_json::Value) -> String {
    if preview.chars().count() <= 256 {
        return preview;
    }
    let truncated: String = preview.chars().take(256).collect();
    let byte_length = serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    format!("{truncated} [truncated, byte_length={byte_length}]")
}

fn why_code_name(code: &crate::types::WhyCode) -> &'static str {
    match code {
        crate::types::WhyCode::CaseAdded => "case_added",
        crate::types::WhyCode::CaseRemoved => "case_removed",
        crate::types::WhyCode::ErrorRaised => "error_raised",
        crate::types::WhyCode::ErrorResolved => "error_resolved",
        crate::types::WhyCode::InputChanged => "input_changed",
        crate::types::WhyCode::ToolCallReordered => "tool_call_reordered",
        crate::types::WhyCode::ToolCallRemoved => "tool_call_removed",
        crate::types::WhyCode::ToolCallAdded => "tool_call_added",
        crate::types::WhyCode::ToolCallArgumentsChanged => "tool_call_arguments_changed",
        crate::types::WhyCode::OutputChanged => "output_changed",
        crate::types::WhyCode::ErrorChanged => "error_changed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::write_json_writer;
    use crate::types::{CaseDiff, DiffSummary, DIFF_KIND, SCHEMA_VERSION};

    fn sample_report() -> DiffReport {
        DiffReport {
            kind: DIFF_KIND.to_string(),
            schema_version: SCHEMA_VERSION,
            summary: DiffSummary {
                unchanged: 0,
                added: 0,
                removed: 0,
                changed: 1,
                regressions: 1,
                improvements: 0,
            },
            cases: vec![CaseDiff {
                id: "capital".to_string(),
                status: CaseStatus::Changed,
                regression: true,
                changed_fields: vec!["output".to_string()],
            }],
        }
    }

    #[test]
    fn format_diff_json_equals_write_json_writer_buffer() {
        let report = sample_report();
        let formatted = format_diff_json(&report).expect("format json");
        let mut buffer = Vec::new();
        write_json_writer(&mut buffer, &report).expect("write json");
        assert_eq!(formatted, String::from_utf8(buffer).expect("utf-8"));
    }

    #[test]
    fn preview_value_truncates_at_256_scalars() {
        let long = "x".repeat(300);
        let preview = preview_value(&crate::types::ValueSlot::present(
            serde_json::Value::String(long.clone()),
        ));
        assert!(preview.contains("[truncated, byte_length="));
        let prefix = preview.split(" [truncated").next().expect("prefix");
        assert_eq!(prefix.chars().count(), 256);
        let short = preview_value(&crate::types::ValueSlot::present(serde_json::json!("ok")));
        assert_eq!(short, "\"ok\"");
        assert_eq!(preview_value(&crate::types::ValueSlot::absent()), "ABSENT");
        assert_eq!(
            preview_value(&crate::types::ValueSlot::present(serde_json::Value::Null)),
            "null"
        );
    }
}
