# LexCompile — Behavioral Diff Tool for AI Agents

> OSS CLI (MIT). Rust. Scaffold the full project.

## Identity

- **Name:** LexCompile
- **Tagline:** Behavioral diff for AI agents
- **Repo:** https://github.com/LatticeAG/lexcompile
- **License:** MIT
- **Framework:** Rust (clap for CLI, serde_json for output, walkdir for test discovery)

## Architecture

A CLI tool with two commands:

### `lattice snapshot`
Runs a golden test suite against an agent and captures baseline behavior into a snapshot file.

**Input:** A JSON file defining test cases:
```json
{
  "tests": [
    { "input": "Write a haiku about AI", "tags": ["creative"] },
    { "input": "What is the capital of France?", "tags": ["factual"] }
  ]
}
```

**Output (snapshot file):** JSON with per-test results including:
- Input, output text, tool calls made, latency, estimated cost
- Metadata: model used, prompt version hash, timestamp

### `lattice diff`
Compares two snapshot files and produces a structured behavioral diff.

**Output (diff file):** JSON with:
- Per-test comparison: output changed? tool calls changed? latency delta?
- Aggregated stats: total regressions, improvements, unchanged
- Summary: "2 regressions, 1 improvement, tool call rate: +12%"

## File Structure

```
lexcompile/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI entry with clap subcommands
│   ├── snapshot.rs       # Snapshot runner logic
│   ├── diff.rs           # Diff computation logic
│   ├── types.rs          # Shared types (TestCase, Snapshot, DiffResult)
│   └── format.rs         # Output formatting (JSON, text)
├── tests/
│   └── fixtures/         # Test fixture files
└── README.md
```

## Key Design Decisions

1. **JSON everywhere** — snapshot files, diff output, test suites. Easy to pipe in CI.
2. **Snapshots are local files** — no server dependency. `lattice snapshot --out baseline.json`
3. **Diff is structural** — compare tool calls, reasoning paths, output similarity (Levenshtein/heuristic), not just string equality.
4. **Cost estimation** — take model name as a param, use a hardcoded lookup table ($/1K tokens).
5. **CLI-first** — no config file. All args are flags.

## Build Order

1. `Cargo.toml` with clap, serde, serde_json, chrono
2. `types.rs` — all data structures
3. `snapshot.rs` — test runner (reads JSON, executes each test through a configurable agent endpoint)
4. `diff.rs` — comparison logic
5. `format.rs` — output formatting
6. `main.rs` — wire up clap subcommands
7. `tests/` — basic integration tests
