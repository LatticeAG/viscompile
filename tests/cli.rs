use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "viscompile-cli-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary test directory");
        Self { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn lattice(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .output()
        .expect("run lattice CLI")
}

fn argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fixture")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("valid JSON output")
}

#[test]
fn compile_is_deterministic_and_snapshot_is_an_alias() {
    let temp = TempDir::new("compile");
    let first_input = temp.file("first.json");
    let second_input = temp.file("second.json");
    let first_output = temp.file("first.snapshot.json");
    let second_output = temp.file("second.snapshot.json");

    write(
        &first_input,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {
      "id": "zeta",
      "input": "Line one\r\nLine two",
      "events": [
        {"type": "final", "output": {"z": 2, "a": 1}}
      ]
    },
    {
      "id": "alpha",
      "input": "Find weather",
      "events": [
        {
          "type": "tool_call",
          "name": "weather.lookup",
          "arguments": "{\"z\":2,\"a\":1}"
        },
        {"type": "final", "output": "Sunny"}
      ]
    }
  ]
}"#,
    );
    write(
        &second_input,
        r#"{
  "schema_version": 1,
  "kind": "latticeag.viscompile.transcript",
  "cases": [
    {
      "id": "alpha",
      "input": "Find weather",
      "events": [
        {"type": "tool_call", "arguments": {"a": 1, "z": 2}, "name": "weather.lookup"},
        {"output": "Sunny", "type": "final"}
      ]
    },
    {
      "id": "zeta",
      "input": "Line one\nLine two",
      "events": [
        {"output": {"a": 1, "z": 2}, "type": "final"}
      ]
    }
  ]
}"#,
    );

    let compile = lattice(&[
        "compile".to_string(),
        "--input".to_string(),
        argument(&first_input),
        "--out".to_string(),
        argument(&first_output),
    ]);
    assert_success(&compile);

    let snapshot = lattice(&[
        "snapshot".to_string(),
        "--input".to_string(),
        argument(&second_input),
        "--out".to_string(),
        argument(&second_output),
    ]);
    assert_success(&snapshot);

    assert_eq!(
        fs::read(&first_output).expect("first snapshot"),
        fs::read(&second_output).expect("second snapshot")
    );
    let snapshot_json = parse_json(&fs::read(&first_output).expect("read snapshot"));
    assert_eq!(snapshot_json["kind"], "latticeag.viscompile.snapshot");
    assert_eq!(snapshot_json["cases"][0]["id"], "alpha");
    assert_eq!(snapshot_json["cases"][1]["id"], "zeta");
    assert_eq!(
        snapshot_json["cases"][0]["tool_calls"][0]["arguments"]["a"],
        1
    );
}

#[test]
fn diff_accepts_raw_and_canonical_inputs_without_false_changes() {
    let temp = TempDir::new("equivalent-diff");
    let baseline = temp.file("baseline.json");
    let target_raw = temp.file("target.json");
    let target_snapshot = temp.file("target.snapshot.json");

    write(
        &baseline,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {"id": "two", "input": "second", "events": [{"type": "final", "output": "B"}]},
    {"id": "one", "input": "first", "events": [
      {"type": "tool_call", "name": "search", "arguments": {"query": "first", "limit": 1}},
      {"type": "final", "output": "A"}
    ]}
  ]
}"#,
    );
    write(
        &target_raw,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {"id": "one", "input": "first", "events": [
      {"arguments": {"limit": 1, "query": "first"}, "name": "search", "type": "tool_call"},
      {"output": "A", "type": "final"}
    ]},
    {"id": "two", "input": "second", "events": [{"output": "B", "type": "final"}]}
  ]
}"#,
    );

    let compile = lattice(&[
        "compile".to_string(),
        "--input".to_string(),
        argument(&target_raw),
        "--out".to_string(),
        argument(&target_snapshot),
    ]);
    assert_success(&compile);

    let diff = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&baseline),
        "--target".to_string(),
        argument(&target_snapshot),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&diff);

    let report = parse_json(&diff.stdout);
    assert_eq!(report["summary"]["unchanged"], 2);
    assert_eq!(report["summary"]["regressions"], 0);
    assert_eq!(report["cases"][0]["id"], "one");
    assert_eq!(report["cases"][1]["id"], "two");
}

#[test]
fn diff_writes_clean_json_and_can_gate_regressions() {
    let temp = TempDir::new("regression");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    let report_path = temp.file("report.json");

    write(&baseline, &single_final_transcript("capital", "Paris."));
    write(&target, &single_final_transcript("capital", "Lyon."));

    let diff = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&baseline),
        "--target".to_string(),
        argument(&target),
        "--out".to_string(),
        argument(&report_path),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&diff);

    let stdout_report = parse_json(&diff.stdout);
    let file_report = parse_json(&fs::read(&report_path).expect("read report"));
    assert_eq!(stdout_report, file_report);
    assert_eq!(stdout_report["summary"]["regressions"], 1);
    assert_eq!(stdout_report["cases"][0]["status"], "changed");
    assert_eq!(stdout_report["cases"][0]["changed_fields"][0], "output");
    assert_eq!(
        stdout_report["cases"][0]["changed_fields"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let gated = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&baseline),
        "--target".to_string(),
        argument(&target),
        "--fail-on-regression".to_string(),
    ]);
    assert_eq!(gated.status.code(), Some(2));
}

#[test]
fn diff_reports_error_resolution_as_an_improvement() {
    let temp = TempDir::new("improvement");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    write(
        &baseline,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {
      "id": "lookup",
      "input": "Find a record",
      "events": [{"type": "error", "message": "upstream unavailable"}]
    }
  ]
}"#,
    );
    write(&target, &single_final_transcript("lookup", "Record found."));

    let diff = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&baseline),
        "--target".to_string(),
        argument(&target),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&diff);
    let report = parse_json(&diff.stdout);
    assert_eq!(report["summary"]["improvements"], 1);
    assert_eq!(report["summary"]["regressions"], 0);
    assert_eq!(report["cases"][0]["status"], "improved");
}

#[test]
fn diff_reports_added_and_removed_cases_without_dropping_them() {
    let temp = TempDir::new("added-removed");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    write(
        &baseline,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {"id": "kept", "input": "keep", "events": [{"type": "final", "output": "same"}]},
    {"id": "removed", "input": "remove", "events": [{"type": "final", "output": "old"}]}
  ]
}"#,
    );
    write(
        &target,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {"id": "added", "input": "add", "events": [{"type": "final", "output": "new"}]},
    {"id": "kept", "input": "keep", "events": [{"type": "final", "output": "same"}]}
  ]
}"#,
    );

    let diff = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&baseline),
        "--target".to_string(),
        argument(&target),
        "--format".to_string(),
        "json".to_string(),
    ]);
    assert_success(&diff);
    let report = parse_json(&diff.stdout);
    assert_eq!(report["summary"]["added"], 1);
    assert_eq!(report["summary"]["removed"], 1);
    assert_eq!(report["summary"]["regressions"], 1);
    assert_eq!(report["cases"][0]["id"], "added");
    assert_eq!(report["cases"][1]["id"], "kept");
    assert_eq!(report["cases"][2]["id"], "removed");
    assert_eq!(report["cases"][2]["status"], "removed");
}

#[test]
fn invalid_transcript_and_format_fail_with_nonzero_status() {
    let temp = TempDir::new("invalid");
    let invalid = temp.file("invalid.json");
    let valid = temp.file("valid.json");
    let output = temp.file("output.json");

    write(
        &invalid,
        r#"{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {"id": "unfinished", "input": "hello", "events": [{"type": "tool_call", "name": "search", "arguments": {}}]}
  ]
}"#,
    );
    write(&valid, &single_final_transcript("valid", "done"));

    let compile = lattice(&[
        "compile".to_string(),
        "--input".to_string(),
        argument(&invalid),
        "--out".to_string(),
        argument(&output),
    ]);
    assert!(!compile.status.success());
    assert!(String::from_utf8_lossy(&compile.stderr).contains("must end"));
    assert!(!output.exists());

    let invalid_format = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&valid),
        "--target".to_string(),
        argument(&valid),
        "--format".to_string(),
        "yaml".to_string(),
    ]);
    assert!(!invalid_format.status.success());
    assert_eq!(invalid_format.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid_format.stderr).contains("invalid value"));
}

#[test]
fn version_prints_crate_version() {
    let output = lattice(&["--version".to_string()]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0.1.0"),
        "version output should include the crate version: {stdout}"
    );
}

#[test]
fn help_and_version_exit_0() {
    for flag in ["--help", "-h", "--version", "-V"] {
        let output = lattice(&[flag.to_string()]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flag} should exit 0\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn clap_usage_errors_exit_1() {
    let temp = TempDir::new("clap-usage");
    let valid = temp.file("valid.json");
    write(&valid, &single_final_transcript("valid", "done"));

    let yaml = lattice(&[
        "diff".to_string(),
        "--baseline".to_string(),
        argument(&valid),
        "--target".to_string(),
        argument(&valid),
        "--format".to_string(),
        "yaml".to_string(),
    ]);
    assert_eq!(yaml.status.code(), Some(1));

    let unknown = lattice(&["not-a-command".to_string()]);
    assert_eq!(unknown.status.code(), Some(1));
}

fn single_final_transcript(id: &str, output: &str) -> String {
    format!(
        r#"{{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {{
      "id": "{id}",
      "input": "test input",
      "events": [{{"type": "final", "output": "{output}"}}]
    }}
  ]
}}"#
    )
}
