# VisCompile Contributor Guide

## Identity

- **Name:** VisCompile
- **Brand:** LatticeAG
- **Tagline:** Behavioral diff for AI agents
- **Repository:** `https://github.com/LatticeAG/viscompile`
- **License:** MIT
- **Language:** Rust
- **Executable:** `lattice`
- **MSRV:** 1.85

## v0.1 architecture

VisCompile is an offline CLI that canonicalizes recorded agent transcripts and diffs them. It does not execute agents or contact an endpoint.

```text
raw transcript JSON
        |
        v
lattice compile or snapshot
        |
        v
canonical snapshot JSON
        |
        +------------------+
        |                  |
        v                  v
  lattice diff       lattice graph
        |                  |
        v                  v
text / JSON / HTML    DOT / Graphviz
summary or comparison change graph
```

The library modules are:

- `canonical.rs` parses streams, validates JSON Schema and semantic invariants, canonicalizes values, hashes snapshot digests, and atomically writes artifacts.
- `diff.rs` joins cases by ID and calculates behavioral changes.
- `compare.rs` builds the evidence comparison: JSON Pointers, tool-call alignment, and why-codes.
- `format.rs` renders terminal-safe text, JSON, and self-contained HTML reports.
- `graph.rs` renders deterministic, escaped Graphviz/DOT change graphs.
- `types.rs` owns the versioned data model, structured validation failures, and typed errors.
- `main.rs` is CLI wiring only.

## Commands

```bash
lattice compile --input transcript.json --out snapshot.json --digest-out snapshot.sha256
lattice snapshot --input transcript.json --out snapshot.json
lattice diff --baseline baseline.json --target target.json --format html --report comparison
lattice graph --baseline baseline.json --target target.json > changes.dot
```

`snapshot` is a compatibility alias for `compile`; both require raw transcripts. `diff` and `graph` accept raw transcripts or canonical snapshots. Use `-` as an input or output path for pipelines (only one diff/graph input may use stdin), `--color auto|always|never` for summary text diffs, `--report summary|comparison` on `diff`, digest flags to pin snapshots, `--max-bytes` (default 67108864) to cap reads, and `--fail-on-regression` when a CI job should return status 2 for regressions. All commands accept mutually exclusive `--verbose` and `--quiet` flags. `lattice --version` prints the crate version.

Exit codes: 0 success, 1 invocation/I/O/schema/digest, 2 `--fail-on-regression`.

## Data contract

Raw transcripts use `latticeag.viscompile.transcript` with `schema_version: 1` and non-empty, unique case IDs. Each case contains an `input` and ordered events. Supported events are `tool_call`, `final`, and `error`; exactly one terminal event is required and must be last.

Canonical snapshots use `latticeag.viscompile.snapshot` with `schema_version: 1`. They sort cases by ID, recursively sort object keys, normalize CRLF to LF, preserve arrays and tool-call order, retain `final.output: null` as a present terminal value, and contain no timestamps or generated metadata. Reject normalized object-key collisions rather than silently dropping data.

Default `lattice diff --report summary` stays a `latticeag.viscompile.diff` document. `--report comparison` is a separate `latticeag.viscompile.comparison` artifact. Comparison names structural change in the recording. It does not recover hidden beliefs.

## Contribution checks

Run these before handing off a change:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Keep integration tests in `tests/` and exercise the compiled `lattice` binary. Keep JSON stdout machine-readable and diagnostics on stderr; stdout text must escape untrusted terminal control bytes. Do not add network behavior, an agent executor, a UI, MCP integration, or vendor-specific log parsing without a new locked build decision.

See [LICENSE](LICENSE) and [SECURITY.md](SECURITY.md).
