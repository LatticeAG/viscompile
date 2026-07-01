use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod diff;
mod format;
mod snapshot;
mod types;

#[derive(Parser)]
#[command(name = "lattice")]
#[command(about = "Behavioral diff for AI agents", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Runs a golden test suite against an agent and captures baseline behavior into a snapshot file.
    Snapshot {
        /// Path to the JSON test suite
        #[arg(short, long)]
        suite: PathBuf,
        /// Path to save the snapshot JSON
        #[arg(short, long)]
        out: PathBuf,
        /// Model to use
        #[arg(short, long, default_value = "gpt-4")]
        model: String,
        /// Agent endpoint
        #[arg(short, long, default_value = "http://localhost:8080")]
        endpoint: String,
    },
    /// Compares two snapshot files and produces a structured behavioral diff.
    Diff {
        /// Path to the baseline snapshot JSON
        #[arg(short, long)]
        baseline: PathBuf,
        /// Path to the target snapshot JSON
        #[arg(short, long)]
        target: PathBuf,
        /// Path to save the diff output JSON (optional)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Output format (json or text)
        #[arg(short, long, default_value = "text")]
        format: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Snapshot {
            suite,
            out,
            model,
            endpoint,
        } => {
            if let Err(e) = snapshot::run_snapshot(suite, out, model, endpoint) {
                eprintln!("Error running snapshot: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Diff {
            baseline,
            target,
            out,
            format,
        } => {
            match diff::compute_diff(baseline, target, out.as_deref()) {
                Ok(diff_result) => {
                    if format == "json" {
                        println!("{}", format::format_diff_json(&diff_result).unwrap());
                    } else {
                        println!("{}", format::format_diff_text(&diff_result));
                    }
                }
                Err(e) => {
                    eprintln!("Error computing diff: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
