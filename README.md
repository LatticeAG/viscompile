# VisCompile

Behavioral diff tooling for AI agents, by LatticeAG.

VisCompile turns recorded agent transcripts into deterministic canonical snapshots, then compares them by stable case ID. It is an offline Rust CLI: it never executes an agent or sends a network request.

## What it does

- Validates versioned transcript JSON against a bundled JSON Schema and the transcript invariants.
- Compiles equivalent recordings into byte-stable canonical snapshots.
- Diffs raw transcripts or snapshots and classifies added, removed, changed, unchanged, regression, and improvement cases.
- Renders a report as human-readable text, machine-readable JSON, or a self-contained HTML document.
- Exports transcript changes as a Graphviz DOT graph.
- Works in shell pipelines with `-` for stdin or stdout.

## Install

```bash
cargo install --path .
lattice --help
```

Or build a release binary:

```bash
cargo build --release
./target/release/lattice --help
```

## Transcript format

Input is a versioned transcript with one or more named cases. Each case ends with exactly one `final` or `error` event.

```json
{
  "kind": "latticeag.viscompile.transcript",
  "schema_version": 1,
  "cases": [
    {
      "id": "capital-question",
      "input": "What is the capital of France?",
      "events": [
        {
          "type": "tool_call",
          "name": "knowledge.lookup",
          "arguments": { "query": "capital of France" }
        },
        {
          "type": "final",
          "output": "Paris."
        }
      ]
    }
  ]
}
```

Raw transcripts are validated against [the bundled JSON Schema](schemas/transcript.schema.json) before semantic validation. The compiler also rejects empty case lists, duplicate or empty IDs, unknown event types, malformed terminal sequences, and invalid JSON-encoded tool arguments.

JSON itself must be UTF-8. For binary tool inputs or outputs, use a JSON representation such as a base64 string or a byte array; VisCompile preserves those payloads while applying the documented key-order and line-ending canonicalization.

## Commands

```text
lattice compile  --input <transcript.json|-> --out <snapshot.json|->
lattice snapshot --input <transcript.json|-> --out <snapshot.json|->
lattice diff     --baseline <document.json|-> --target <document.json|-> \
                 [--format text|json|html] [--out <report.json>] \
                 [--color auto|always|never] [--fail-on-regression]
lattice graph    --baseline <document.json|-> --target <document.json|-> \
                 [--out <graph.dot|->] [--fail-on-regression]
```

`snapshot` is a compatibility alias for `compile`. Both write a `latticeag.viscompile.snapshot` version 1 document. Cases and object keys are sorted deterministically, CRLF line endings are normalized, arrays and tool-call order are retained, and no generated metadata is added.

`diff` and `graph` accept either raw transcripts or snapshots. They may read one side from stdin (`-`), but cannot read both baseline and target from the same stdin stream.

All commands support mutually exclusive `--verbose` and `--quiet` flags. Routine diagnostics and save messages go to stderr; use `--quiet` in a pipeline and `--verbose` to see additional progress details.

## Diff formats

Text is the default interactive format. It is colorized when output goes to a terminal; use `--color auto`, `--color always`, or `--color never` to control ANSI color. Color only affects text output—JSON, HTML, and DOT remain machine-readable.

```bash
lattice diff \
  --baseline before.snapshot.json \
  --target after.transcript.json \
  --format text \
  --color always
```

For CI or other programs, request JSON. Stdout is a single JSON document, while `--out` always writes the JSON report regardless of the display format.

```bash
lattice diff \
  --baseline before.snapshot.json \
  --target after.snapshot.json \
  --format json \
  --out behavior-diff.json \
  --fail-on-regression
```

An HTML report is a standalone document suitable for a browser or CI artifact. `--out` still receives the canonical JSON report in this mode.

```bash
lattice diff \
  --baseline before.snapshot.json \
  --target after.snapshot.json \
  --format html > behavior-diff.html
```

Case IDs are the diff identity. A target-only case is an added coverage case. A baseline-only case is a regression. Changed input, final output, error, or ordered tool-call behavior is conservatively a regression. A baseline error resolved to a final output is an improvement.

By default, a valid diff exits zero even when regressions are present. `--fail-on-regression` exits with status 2 when the report contains one or more regressions.

## Graphviz export

`lattice graph` writes a deterministic DOT graph to stdout. It includes a report node and one node per compared case, with each case status represented in the graph.

```bash
lattice graph \
  --baseline before.snapshot.json \
  --target after.snapshot.json \
  --quiet > changes.dot

dot -Tsvg changes.dot > changes.svg
```

Pass `--out changes.dot` to save the same DOT artifact while retaining stdout output. `--out -` writes it once to stdout.

## Pipeline examples

Compile a recording arriving on stdin directly into a snapshot file:

```bash
agent-recorder --json | lattice compile --quiet --input - --out baseline.snapshot.json
```

Keep the snapshot itself in the pipeline:

```bash
cat before.transcript.json \
  | lattice compile --quiet --input - --out - \
  > before.snapshot.json
```

Compare a stored baseline with a target streamed on stdin:

```bash
agent-recorder --json \
  | lattice diff \
      --quiet \
      --baseline baseline.snapshot.json \
      --target - \
      --format json \
  | jq '.summary'
```

VisCompile parses JSON in memory, so provision memory appropriate to the largest transcript or snapshot you process. It does not impose an artificial transcript-size limit, and validation or file-writing failures do not leave a partial output file artifact.

## Development

Run the full local verification suite before handing off a change:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Integration tests live in `tests/` and invoke the compiled `lattice` binary.

## License

MIT.

**LatticeAG** - *Agents, together.*
