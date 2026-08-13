use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "viscompile-extended-cli-{label}-{}-{counter}",
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

fn lattice(args: Vec<String>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .output()
        .expect("run lattice CLI")
}

fn lattice_with_stdin(args: Vec<String>, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start lattice CLI");

    child
        .stdin
        .take()
        .expect("open command stdin")
        .write_all(input)
        .expect("write transcript to stdin");
    child.wait_with_output().expect("wait for lattice CLI")
}

fn argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_bytes(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("write fixture");
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

fn transcript(cases: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "kind": "latticeag.viscompile.transcript",
        "schema_version": 1,
        "cases": cases,
    }))
    .expect("serialize transcript")
}

fn final_case(id: &str, input: Value, output: Value) -> Value {
    json!({
        "id": id,
        "input": input,
        "events": [{"type": "final", "output": output}],
    })
}

fn single_final_transcript(id: &str, output: Value) -> Vec<u8> {
    transcript(vec![final_case(id, json!("test input"), output)])
}

#[test]
fn compile_reads_stdin_and_writes_a_snapshot_to_stdout() {
    let output = lattice_with_stdin(
        vec![
            "compile".into(),
            "--quiet".into(),
            "--input".into(),
            "-".into(),
            "--out".into(),
            "-".into(),
        ],
        &single_final_transcript("from-stdin", json!({"answer": "Paris"})),
    );

    assert_success(&output);
    assert!(
        output.stderr.is_empty(),
        "quiet pipeline mode must not contaminate stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let snapshot = parse_json(&output.stdout);
    assert_eq!(snapshot["kind"], "latticeag.viscompile.snapshot");
    assert_eq!(snapshot["cases"][0]["id"], "from-stdin");
    assert_eq!(snapshot["cases"][0]["output"]["answer"], "Paris");
}

#[test]
fn diff_accepts_one_streamed_input_and_keeps_json_stdout_clean() {
    let temp = TempDir::new("diff-stdin");
    let baseline = temp.file("baseline.json");
    write_bytes(
        &baseline,
        &single_final_transcript("streamed-case", json!("before")),
    );

    let output = lattice_with_stdin(
        vec![
            "diff".into(),
            "--quiet".into(),
            "--baseline".into(),
            argument(&baseline),
            "--target".into(),
            "-".into(),
            "--format".into(),
            "json".into(),
        ],
        &single_final_transcript("streamed-case", json!("after")),
    );

    assert_success(&output);
    assert!(
        output.stderr.is_empty(),
        "quiet JSON pipeline must not emit diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_json(&output.stdout);
    assert_eq!(report["kind"], "latticeag.viscompile.diff");
    assert_eq!(report["summary"]["regressions"], 1);
    assert_eq!(report["cases"][0]["id"], "streamed-case");
}

#[test]
fn diff_html_is_a_complete_document_and_out_remains_json() {
    let temp = TempDir::new("html");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    let report_path = temp.file("report.json");
    write_bytes(
        &baseline,
        &single_final_transcript("changed-case", json!("before")),
    );
    write_bytes(
        &target,
        &single_final_transcript("changed-case", json!("after")),
    );

    let output = lattice(vec![
        "diff".into(),
        "--baseline".into(),
        argument(&baseline),
        "--target".into(),
        argument(&target),
        "--format".into(),
        "html".into(),
        "--out".into(),
        argument(&report_path),
    ]);

    assert_success(&output);
    let html = String::from_utf8(output.stdout).expect("HTML is UTF-8");
    let lower_html = html.to_ascii_lowercase();
    assert!(
        lower_html.trim_start().starts_with("<!doctype html>"),
        "HTML output should be a standalone document: {html}"
    );
    assert!(
        lower_html.contains("<html"),
        "HTML document has an html root"
    );
    assert!(html.contains("changed-case"), "HTML includes the case ID");
    assert!(
        lower_html.contains("changed"),
        "HTML includes the case status"
    );

    let report = parse_json(&fs::read(&report_path).expect("read JSON report"));
    assert_eq!(report["kind"], "latticeag.viscompile.diff");
    assert_eq!(report["summary"]["regressions"], 1);
}

#[test]
fn graph_outputs_dot_for_all_change_kinds() {
    let temp = TempDir::new("graph");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    write_bytes(
        &baseline,
        &transcript(vec![
            final_case("changed", json!("same input"), json!("before")),
            final_case("removed", json!("old input"), json!("old output")),
        ]),
    );
    write_bytes(
        &target,
        &transcript(vec![
            final_case("added", json!("new input"), json!("new output")),
            final_case("changed", json!("same input"), json!("after")),
        ]),
    );

    let output = lattice(vec![
        "graph".into(),
        "--quiet".into(),
        "--baseline".into(),
        argument(&baseline),
        "--target".into(),
        argument(&target),
    ]);

    assert_success(&output);
    assert!(
        output.stderr.is_empty(),
        "quiet graph output must not emit diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dot = String::from_utf8(output.stdout).expect("DOT is UTF-8");
    assert!(
        dot.trim_start().starts_with("digraph"),
        "graph command should emit DOT: {dot}"
    );
    for case_id in ["added", "changed", "removed"] {
        assert!(dot.contains(case_id), "DOT is missing `{case_id}`: {dot}");
    }
    assert!(dot.contains("->"), "DOT graph contains graph edges: {dot}");

    let graph_path = temp.file("changes.dot");
    let saved_output = lattice(vec![
        "graph".into(),
        "--quiet".into(),
        "--baseline".into(),
        argument(&baseline),
        "--target".into(),
        argument(&target),
        "--out".into(),
        argument(&graph_path),
    ]);
    assert_success(&saved_output);
    assert_eq!(
        saved_output.stdout,
        fs::read(&graph_path).expect("read saved DOT graph"),
        "graph artifact and stdout stay identical"
    );
}

#[test]
fn text_color_modes_render_the_same_semantic_report() {
    let temp = TempDir::new("color-modes");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    write_bytes(
        &baseline,
        &single_final_transcript("color-case", json!("before")),
    );
    write_bytes(
        &target,
        &single_final_transcript("color-case", json!("after")),
    );

    for color in ["auto", "always", "never"] {
        let output = lattice(vec![
            "diff".into(),
            "--quiet".into(),
            "--baseline".into(),
            argument(&baseline),
            "--target".into(),
            argument(&target),
            "--format".into(),
            "text".into(),
            "--color".into(),
            color.into(),
        ]);
        assert_success(&output);
        let rendered = String::from_utf8_lossy(&output.stdout);
        assert!(
            rendered.contains("VisCompile Behavioral Diff"),
            "{color} output retains the text report heading"
        );
        assert!(
            rendered.contains("color-case"),
            "{color} output retains the case ID"
        );
    }
}

#[test]
fn verbose_and_quiet_apply_to_every_subcommand() {
    let temp = TempDir::new("verbosity");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    let compile_out = temp.file("compile.snapshot.json");
    let snapshot_out = temp.file("snapshot.snapshot.json");
    write_bytes(
        &baseline,
        &single_final_transcript("verbosity", json!("before")),
    );
    write_bytes(
        &target,
        &single_final_transcript("verbosity", json!("after")),
    );

    for (command, output_path) in [("compile", &compile_out), ("snapshot", &snapshot_out)] {
        let quiet = lattice(vec![
            command.into(),
            "--quiet".into(),
            "--input".into(),
            argument(&baseline),
            "--out".into(),
            argument(output_path),
        ]);
        assert_success(&quiet);
        assert!(
            quiet.stderr.is_empty(),
            "{command} --quiet emitted diagnostics: {}",
            String::from_utf8_lossy(&quiet.stderr)
        );

        let verbose = lattice(vec![
            command.into(),
            "--verbose".into(),
            "--input".into(),
            argument(&baseline),
            "--out".into(),
            argument(output_path),
        ]);
        assert_success(&verbose);
        assert!(
            !verbose.stderr.is_empty(),
            "{command} --verbose should describe work on stderr"
        );
    }

    for command in ["diff", "graph"] {
        let mut quiet_args = vec![
            command.to_string(),
            "--quiet".into(),
            "--baseline".into(),
            argument(&baseline),
            "--target".into(),
            argument(&target),
        ];
        if command == "diff" {
            quiet_args.extend(["--format".into(), "json".into()]);
        }
        let quiet = lattice(quiet_args);
        assert_success(&quiet);
        assert!(
            quiet.stderr.is_empty(),
            "{command} --quiet emitted diagnostics: {}",
            String::from_utf8_lossy(&quiet.stderr)
        );

        let mut verbose_args = vec![
            command.to_string(),
            "--verbose".into(),
            "--baseline".into(),
            argument(&baseline),
            "--target".into(),
            argument(&target),
        ];
        if command == "diff" {
            verbose_args.extend(["--format".into(), "json".into()]);
        }
        let verbose = lattice(verbose_args);
        assert_success(&verbose);
        assert!(
            !verbose.stderr.is_empty(),
            "{command} --verbose should describe work on stderr"
        );
    }
}

#[test]
fn empty_and_non_utf8_transcripts_fail_cleanly() {
    let temp = TempDir::new("invalid-input");
    let empty = temp.file("empty.json");
    let output_path = temp.file("empty.snapshot.json");
    write_bytes(
        &empty,
        br#"{"kind":"latticeag.viscompile.transcript","schema_version":1,"cases":[]}"#,
    );

    let empty_output = lattice(vec![
        "compile".into(),
        "--input".into(),
        argument(&empty),
        "--out".into(),
        argument(&output_path),
    ]);
    assert!(
        !empty_output.status.success(),
        "an empty transcript must not be accepted"
    );
    assert!(
        empty_output.stdout.is_empty(),
        "failed compile has no stdout"
    );
    assert!(
        !empty_output.stderr.is_empty(),
        "empty transcript reports an actionable error"
    );
    assert!(
        String::from_utf8_lossy(&empty_output.stderr).contains("JSON Schema"),
        "empty transcript is rejected by JSON Schema validation"
    );
    assert!(
        !output_path.exists(),
        "failed compile must not leave an output file"
    );

    let binary_output = lattice_with_stdin(
        vec![
            "compile".into(),
            "--input".into(),
            "-".into(),
            "--out".into(),
            "-".into(),
        ],
        &[0xff, 0xfe, 0x00, b'{'],
    );
    assert!(
        !binary_output.status.success(),
        "non-UTF-8 input must fail rather than panic"
    );
    assert!(
        binary_output.stdout.is_empty(),
        "failed stdin compile has no partial stdout document"
    );
    assert!(
        !binary_output.stderr.is_empty(),
        "non-UTF-8 input produces a diagnostic"
    );
}

#[test]
fn binary_payload_representations_are_preserved() {
    let temp = TempDir::new("binary-payload");
    let input = temp.file("binary.json");
    let output = temp.file("binary.snapshot.json");
    let payload = json!({
        "content_type": "application/octet-stream",
        "base64": "AP+AQQ==",
        "bytes": [0, 255, 128, 65],
        "nul_delimited": "before\u{0}after",
    });
    write_bytes(
        &input,
        &transcript(vec![final_case(
            "binary-payload",
            json!({"attachment": payload.clone()}),
            payload.clone(),
        )]),
    );

    let command_output = lattice(vec![
        "compile".into(),
        "--quiet".into(),
        "--input".into(),
        argument(&input),
        "--out".into(),
        argument(&output),
    ]);
    assert_success(&command_output);
    let snapshot = parse_json(&fs::read(&output).expect("read snapshot"));
    assert_eq!(snapshot["cases"][0]["input"]["attachment"], payload);
    assert_eq!(
        snapshot["cases"][0]["output"]["bytes"],
        json!([0, 255, 128, 65])
    );
    assert_eq!(
        snapshot["cases"][0]["output"]["nul_delimited"],
        "before\u{0}after"
    );
}

#[test]
fn large_transcripts_compile_without_truncation() {
    let temp = TempDir::new("large");
    let input = temp.file("large.json");
    let output = temp.file("large.snapshot.json");
    let large_value = "x".repeat(1024 * 1024);
    write_bytes(
        &input,
        &transcript(vec![final_case(
            "large-case",
            json!({"body": large_value.clone()}),
            json!({"body": large_value.clone()}),
        )]),
    );

    let command_output = lattice(vec![
        "compile".into(),
        "--quiet".into(),
        "--input".into(),
        argument(&input),
        "--out".into(),
        argument(&output),
    ]);
    assert_success(&command_output);

    let snapshot = parse_json(&fs::read(&output).expect("read large snapshot"));
    assert_eq!(
        snapshot["cases"][0]["output"]["body"]
            .as_str()
            .expect("large output string")
            .len(),
        large_value.len()
    );
    assert_eq!(snapshot["cases"][0]["output"]["body"], large_value);
}

#[test]
fn null_final_output_round_trips_through_a_snapshot_and_diff() {
    let temp = TempDir::new("null-output");
    let raw = temp.file("raw.json");
    let snapshot = temp.file("snapshot.json");
    let rejected_output = temp.file("rejected.json");
    write_bytes(&raw, &single_final_transcript("null-output", Value::Null));

    let compile = lattice(vec![
        "compile".into(),
        "--quiet".into(),
        "--input".into(),
        argument(&raw),
        "--out".into(),
        argument(&snapshot),
    ]);
    assert_success(&compile);

    let saved = parse_json(&fs::read(&snapshot).expect("read snapshot"));
    let saved_case = saved["cases"][0].as_object().expect("snapshot case object");
    assert!(
        saved_case.contains_key("output"),
        "a null final output must remain present in the snapshot"
    );
    assert!(saved_case["output"].is_null());

    let diff = lattice(vec![
        "diff".into(),
        "--quiet".into(),
        "--baseline".into(),
        argument(&snapshot),
        "--target".into(),
        argument(&raw),
        "--format".into(),
        "json".into(),
    ]);
    assert_success(&diff);
    let report = parse_json(&diff.stdout);
    assert_eq!(report["summary"]["unchanged"], 1);
    assert_eq!(report["summary"]["regressions"], 0);

    let rejected = lattice(vec![
        "compile".into(),
        "--input".into(),
        argument(&snapshot),
        "--out".into(),
        argument(&rejected_output),
    ]);
    assert!(
        !rejected.status.success(),
        "compile must require a raw transcript rather than recompile a snapshot"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("raw transcript"),
        "compile rejection should explain the accepted input type"
    );
    assert!(!rejected_output.exists());
}

#[test]
fn unsafe_case_ids_are_escaped_and_key_normalization_collisions_fail_closed() {
    let temp = TempDir::new("unsafe-values");
    let baseline = temp.file("baseline.json");
    let target = temp.file("target.json");
    let collision = temp.file("collision.json");
    let collision_out = temp.file("collision.snapshot.json");
    let unsafe_id = "<script>\u{1b}[31m\ncase";
    write_bytes(
        &baseline,
        &single_final_transcript(unsafe_id, json!("before")),
    );
    write_bytes(&target, &single_final_transcript(unsafe_id, json!("after")));

    let text = lattice(vec![
        "diff".into(),
        "--quiet".into(),
        "--baseline".into(),
        argument(&baseline),
        "--target".into(),
        argument(&target),
        "--color".into(),
        "never".into(),
    ]);
    assert_success(&text);
    let text = String::from_utf8(text.stdout).expect("text output is UTF-8");
    assert!(
        !text.contains('\u{1b}'),
        "terminal escape must not be emitted"
    );
    assert!(
        text.contains("\\u{1b}"),
        "terminal escape is rendered visibly"
    );
    assert!(
        text.contains("\\ncase"),
        "newlines in IDs are rendered visibly"
    );

    let html = lattice(vec![
        "diff".into(),
        "--quiet".into(),
        "--baseline".into(),
        argument(&baseline),
        "--target".into(),
        argument(&target),
        "--format".into(),
        "html".into(),
    ]);
    assert_success(&html);
    let html = String::from_utf8(html.stdout).expect("HTML output is UTF-8");
    assert!(!html.contains("<script>"), "HTML must escape case IDs");
    assert!(html.contains("&lt;script&gt;"));
    assert!(
        !html.contains('\u{1b}'),
        "HTML must not embed control bytes"
    );

    let mut colliding_input = Map::new();
    colliding_input.insert("line\r\nkey".to_string(), json!("first"));
    colliding_input.insert("line\nkey".to_string(), json!("second"));
    write_bytes(
        &collision,
        &transcript(vec![final_case(
            "collision",
            Value::Object(colliding_input),
            json!("unused"),
        )]),
    );
    let collision_result = lattice(vec![
        "compile".into(),
        "--input".into(),
        argument(&collision),
        "--out".into(),
        argument(&collision_out),
    ]);
    assert!(!collision_result.status.success());
    assert!(
        String::from_utf8_lossy(&collision_result.stderr).contains("keys collide"),
        "key collision should be an actionable validation error"
    );
    assert!(!collision_out.exists());
}
