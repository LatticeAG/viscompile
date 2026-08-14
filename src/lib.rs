//! VisCompile is an offline behavioral diff for recorded AI agent transcripts.
//!
//! It never opens a network socket and never executes an agent. Compile a versioned
//! transcript into a canonical snapshot, then diff or graph two snapshots (or raw
//! transcripts) by case id.

pub mod canonical;
pub mod compare;
pub mod diff;
pub mod format;
pub mod graph;
pub mod types;

pub use canonical::{
    canonicalize_document, canonicalize_transcript, canonicalize_value, load_snapshot,
    load_transcript, snapshot_digest, write_json_file, write_json_writer,
};
pub use compare::compare_snapshots;
pub use diff::diff_snapshots;
pub use format::{
    format_comparison_html, format_comparison_text, format_diff_html, format_diff_json,
    format_diff_text,
};
pub use graph::format_diff_dot;
pub use types::{
    escape_untrusted_text, CanonicalCase, CanonicalSnapshot, ComparisonReport, DiffReport,
    VisCompileError, COMPARISON_KIND, DIFF_KIND, SCHEMA_VERSION, SNAPSHOT_KIND, TRANSCRIPT_KIND,
};
