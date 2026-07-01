# VisCompile

Behavioral diff tool for AI agents -- catch regressions before they ship.

**by LatticeAG**

[![Rust](https://img.shields.io/badge/Rust-1.80-dea584?logo=rust&logoColor=white&labelColor=black)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-black.svg?logo=opensourceinitiative&logoColor=white)](./LICENSE)
[![Open Source](https://img.shields.io/badge/Open-Source-black.svg?logo=github&logoColor=white)](https://github.com/LatticeAG/viscompile)
[![CLI](https://img.shields.io/badge/CLI-lattice-amber?logo=terminal&logoColor=white&labelColor=black)](.)

---

## Overview

> **VisCompile snapshots agent behavior against a golden test suite, then diffs it after any change.**
> Run before and after a prompt edit, model swap, or tool update -- get a structured report of what changed, what regressed, and what improved.

Prompt changes, model swaps, and tool updates silently change agent behavior. Teams discover regressions in production because there is no safety net -- no structured way to ask "what changed?" before deploying.

VisCompile is that safety net. A CLI-first tool that treats agent behavior as something you can version, diff, and gate on.

```
                          ┌─────────────────┐
                          │  Golden Test     │
                          │  Suite (JSON)    │
                          └────────┬────────┘
                                   │
                    ┌──────────────┴──────────────┐
                    │                              │
                    ▼                              ▼
            ┌──────────────┐              ┌──────────────┐
            │  Snapshot A  │              │  Snapshot B  │
            │ (before)     │              │ (after)      │
            └──────┬───────┘              └──────┬───────┘
                   │                             │
                   └──────────┬──────────────────┘
                              ▼
                     ┌─────────────────┐
                     │  lattice diff   │
                     │  "2 regressions │
                     │  1 improvement" │
                     └─────────────────┘
```

Built in Rust for speed. OSS CLI (MIT). SaaS dashboard for team history and CI/CD gates coming.

---

## Core Features

| Feature | Description |
|---------|-------------|
| **Snapshot** | Run golden test suites against your agent, capture outputs, tool calls, latency, and cost |
| **Diff** | Compare two snapshots -- structured behavioral delta with regression detection |
| **Structural Comparison** | Not just string equality -- compares tool calls, reasoning paths, output similarity |
| **Cost & Latency Tracking** | Built-in estimation per model, tracked across versions |
| **JSON Everywhere** | Test suites, snapshots, and diffs are all JSON -- pipe into any CI pipeline |
| **CI/CD Ready** | Exit codes and machine-readable output for regression gates |

---

## Quick Start

```bash
# Install the CLI
cargo install --git https://github.com/LatticeAG/viscompile

# Or build from source
git clone https://github.com/LatticeAG/viscompile.git
cd viscompile
cargo build --release
./target/release/lattice --help
```

### Define a test suite

```bash
cat > tests/golden.json << 'EOF'
{
  "tests": [
    { "input": "Write a haiku about AI", "tags": ["creative"] },
    { "input": "What is the capital of France?", "tags": ["factual"] }
  ]
}
EOF
```

### Snapshot current behavior

```bash
lattice snapshot --tests tests/golden.json --out baseline.json
```

### Make a change, then diff

```bash
# ... edit your prompt or swap your model ...

lattice diff --baseline baseline.json --out diff.json
# Output: "2 regressions, 1 improvement, tool call rate: +12%"
```

---

## Commands

| Command | Description |
|---------|-------------|
| `lattice snapshot` | Run a golden test suite and save a snapshot file |
| `lattice diff` | Compare two snapshots and output the behavioral diff |
| `lattice --help` | Full usage reference |

### Snapshot options

| Flag | Description |
|------|-------------|
| `--tests <path>` | Path to golden test suite JSON |
| `--out <path>` | Output snapshot file path |
| `--model <name>` | Model identifier for cost estimation |
| `--prompt-hash <hash>` | Optional prompt version identifier |

### Diff options

| Flag | Description |
|------|-------------|
| `--baseline <path>` | Baseline snapshot file |
| `--target <path>` | Target snapshot file (defaults to latest) |
| `--out <path>` | Output diff file path |
| `--format <text\|json>` | Output format (default: text) |

---

## File Structure

```
viscompile/
├── Cargo.toml              # Rust dependencies
├── src/
│   ├── main.rs             # CLI entry (clap subcommands)
│   ├── snapshot.rs         # Test runner logic
│   ├── diff.rs             # Structural comparison engine
│   ├── types.rs            # Shared data structures
│   └── format.rs           # Output formatters (JSON, text)
├── tests/
│   └── fixtures/           # Sample golden test suites
└── README.md
```

---

## Integration

VisCompile works with any agent framework -- model-agnostic. Pipe the JSON output into your CI pipeline:

```bash
# CI regression gate
lattice diff --baseline baseline.json --format json | jq '.regressions'
if [ $? -gt 0 ]; then exit 1; fi
```

---

## Roadmap

| Phase | Feature | Status |
|:-----:|---------|:------:|
| 1 | CLI snapshot + diff commands | Done |
| 2 | Structural tool call comparison | Done |
| 3 | Cost and latency estimation | Done |
| 4 | SaaS snapshot history + trending | Planned |
| 5 | CI/CD bot (GitHub Actions) | Planned |
| 6 | Regression threshold gates | Planned |

---

## Links

- **Product page:** [latticeag.vercel.app/products/viscompile](https://latticeag.vercel.app/products/viscompile)
- **Source code:** [github.com/LatticeAG/viscompile](https://github.com/LatticeAG/viscompile)
- **LatticeAG:** [latticeag.vercel.app](https://latticeag.vercel.app)

---

## License

MIT -- see [LICENSE](./LICENSE).

---

**LatticeAG** -- *Agents, together.*
[github.com/LatticeAG/viscompile](https://github.com/LatticeAG/viscompile)
