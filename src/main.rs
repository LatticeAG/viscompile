use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use viscompile::canonical::{
    load_snapshot, load_snapshot_reader, load_transcript, load_transcript_reader, write_json_file,
    write_json_writer, write_text_file,
};
use viscompile::diff::diff_snapshots;
use viscompile::format::{escape_text, format_diff_html};
use viscompile::graph::format_diff_dot;
use viscompile::types::{CaseStatus, DiffReport, Result, VisCompileError};

#[derive(Debug, Parser)]
#[command(name = "lattice")]
#[command(about = "VisCompile by LatticeAG - behavioral diff for AI agents")]
struct Cli {
    /// Include progress and summary diagnostics on stderr.
    #[arg(
        short,
        long,
        global = true,
        action = ArgAction::SetTrue,
        conflicts_with = "quiet"
    )]
    verbose: bool,
    /// Suppress routine diagnostics; errors are still written to stderr.
    #[arg(
        short,
        long,
        global = true,
        action = ArgAction::SetTrue,
        conflicts_with = "verbose"
    )]
    quiet: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Compile a recorded agent transcript into a canonical snapshot.
    Compile {
        /// Path to a versioned transcript JSON file, or `-` for stdin.
        #[arg(short, long)]
        input: PathBuf,
        /// Path for the canonical snapshot JSON file, or `-` for stdout.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Compatibility alias for `compile`.
    Snapshot {
        /// Path to a versioned transcript JSON file, or `-` for stdin.
        #[arg(short, long)]
        input: PathBuf,
        /// Path for the canonical snapshot JSON file, or `-` for stdout.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Compare two raw transcripts or canonical snapshots.
    Diff {
        /// Path to the baseline transcript or canonical snapshot, or `-` for stdin.
        #[arg(short, long)]
        baseline: PathBuf,
        /// Path to the target transcript or canonical snapshot, or `-` for stdin.
        #[arg(short, long)]
        target: PathBuf,
        /// Optional path for the JSON diff report; `-` requires `--format json`.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Display format for stdout.
        #[arg(short, long, value_enum, default_value_t = DiffFormat::Text)]
        format: DiffFormat,
        /// Color mode for human-readable text output.
        #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
        color: ColorMode,
        /// Exit with status 2 when the diff contains regressions.
        #[arg(long)]
        fail_on_regression: bool,
    },
    /// Render transcript changes as a DOT/Graphviz graph.
    Graph {
        /// Path to the baseline transcript or canonical snapshot, or `-` for stdin.
        #[arg(short, long)]
        baseline: PathBuf,
        /// Path to the target transcript or canonical snapshot, or `-` for stdin.
        #[arg(short, long)]
        target: PathBuf,
        /// Optional path for a DOT graph; `-` leaves it on stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Exit with status 2 when the graph contains regressions.
        #[arg(long)]
        fail_on_regression: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DiffFormat {
    Text,
    Json,
    Html,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl From<ColorMode> for ColorChoice {
    fn from(mode: ColorMode) -> Self {
        match mode {
            ColorMode::Auto => Self::Auto,
            ColorMode::Always => Self::Always,
            ColorMode::Never => Self::Never,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

impl Verbosity {
    fn from_flags(verbose: bool, quiet: bool) -> Self {
        if quiet {
            Self::Quiet
        } else if verbose {
            Self::Verbose
        } else {
            Self::Normal
        }
    }

    fn status(self, message: impl AsRef<str>) {
        if self != Self::Quiet {
            eprintln!("{}", message.as_ref());
        }
    }

    fn detail(self, message: impl AsRef<str>) {
        if self == Self::Verbose {
            eprintln!("{}", message.as_ref());
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("lattice: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let verbosity = Verbosity::from_flags(cli.verbose, cli.quiet);

    match cli.command {
        Commands::Compile { input, out } | Commands::Snapshot { input, out } => {
            compile(&input, &out, verbosity)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Diff {
            baseline,
            target,
            out,
            format,
            color,
            fail_on_regression,
        } => {
            let report = load_diff(&baseline, &target, verbosity)?;
            write_diff_report(&report, out.as_deref(), format, color, verbosity)?;
            exit_for_regressions(report.summary.regressions, fail_on_regression)
        }
        Commands::Graph {
            baseline,
            target,
            out,
            fail_on_regression,
        } => {
            let report = load_diff(&baseline, &target, verbosity)?;
            write_graph(&report, out.as_deref(), verbosity)?;
            exit_for_regressions(report.summary.regressions, fail_on_regression)
        }
    }
}

fn exit_for_regressions(regressions: u64, fail_on_regression: bool) -> Result<ExitCode> {
    if fail_on_regression && regressions > 0 {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn compile(input: &Path, out: &Path, verbosity: Verbosity) -> Result<()> {
    verbosity.detail(format!("Reading transcript from {}", display_path(input)));
    let snapshot = load_transcript_input(input)?;
    if is_stdio_path(out) {
        write_json_stdout(&snapshot)?;
        verbosity.status("Canonical snapshot written to stdout");
    } else {
        write_json_file(out, &snapshot)?;
        verbosity.status(format!("Canonical snapshot saved to {}", display_path(out)));
    }
    Ok(())
}

fn load_diff(baseline: &Path, target: &Path, verbosity: Verbosity) -> Result<DiffReport> {
    if is_stdio_path(baseline) && is_stdio_path(target) {
        return Err(VisCompileError::validation(
            "only one of `--baseline` and `--target` may be `-` because stdin can be read once",
        ));
    }

    verbosity.detail(format!("Reading baseline from {}", display_path(baseline)));
    let baseline_snapshot = load_input(baseline)?;
    verbosity.detail(format!("Reading target from {}", display_path(target)));
    let target_snapshot = load_input(target)?;
    let report = diff_snapshots(&baseline_snapshot, &target_snapshot);
    verbosity.detail(format!(
        "Diff complete: {} cases, {} regressions, {} improvements",
        report.cases.len(),
        report.summary.regressions,
        report.summary.improvements
    ));
    Ok(report)
}

fn load_input(path: &Path) -> Result<viscompile::types::CanonicalSnapshot> {
    if is_stdio_path(path) {
        let stdin = io::stdin();
        load_snapshot_reader(stdin.lock())
    } else {
        load_snapshot(path)
    }
}

fn load_transcript_input(path: &Path) -> Result<viscompile::types::CanonicalSnapshot> {
    if is_stdio_path(path) {
        let stdin = io::stdin();
        load_transcript_reader(stdin.lock())
    } else {
        load_transcript(path)
    }
}

fn write_diff_report(
    report: &DiffReport,
    output_path: Option<&Path>,
    format: DiffFormat,
    color: ColorMode,
    verbosity: Verbosity,
) -> Result<()> {
    let output_to_stdout = output_path.is_some_and(is_stdio_path);
    if output_to_stdout && !matches!(format, DiffFormat::Json) {
        return Err(VisCompileError::validation(
            "`diff --out -` requires `--format json` so stdout remains a single document",
        ));
    }

    if let Some(path) = output_path.filter(|path| !is_stdio_path(path)) {
        write_json_file(path, report)?;
        verbosity.status(format!("Diff report saved to {}", display_path(path)));
    }

    match format {
        DiffFormat::Text => write_colored_diff_text(report, color),
        DiffFormat::Json => write_json_stdout(report),
        DiffFormat::Html => write_stdout(&format_diff_html(report)),
    }
}

fn write_graph(
    report: &DiffReport,
    output_path: Option<&Path>,
    verbosity: Verbosity,
) -> Result<()> {
    let dot = format_diff_dot(report);
    if let Some(path) = output_path.filter(|path| !is_stdio_path(path)) {
        write_text_file(path, &dot)?;
        verbosity.status(format!("DOT graph saved to {}", display_path(path)));
    }
    write_stdout(&dot)
}

fn write_colored_diff_text(report: &DiffReport, color: ColorMode) -> Result<()> {
    let mut stdout = StandardStream::stdout(color.into());
    write_styled(
        &mut stdout,
        color_spec(Color::Cyan, true),
        "VisCompile Behavioral Diff\n==========================\n\n",
    )?;

    let summary = &report.summary;
    let summary_color = if summary.regressions > 0 {
        Color::Red
    } else {
        Color::Green
    };
    write_styled(
        &mut stdout,
        color_spec(summary_color, true),
        &format!(
            "{} regressions, {} improvements, {} changed, {} added, {} removed, {} unchanged\n",
            summary.regressions,
            summary.improvements,
            summary.changed,
            summary.added,
            summary.removed,
            summary.unchanged
        ),
    )?;
    stdout.write_all(b"\nCases:\n")?;

    for case in &report.cases {
        stdout.write_all(b"- [")?;
        write_styled(
            &mut stdout,
            color_spec(status_color(&case.status), case.regression),
            status_name(&case.status),
        )?;
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
        writeln!(
            &mut stdout,
            "] {}: {} ({gate})",
            escape_text(&case.id),
            fields
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn color_spec(color: Color, bold: bool) -> ColorSpec {
    let mut spec = ColorSpec::new();
    spec.set_fg(Some(color)).set_bold(bold);
    spec
}

fn write_styled(output: &mut StandardStream, color: ColorSpec, contents: &str) -> Result<()> {
    output.set_color(&color)?;
    output.write_all(contents.as_bytes())?;
    output.reset()?;
    Ok(())
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

fn status_color(status: &CaseStatus) -> Color {
    match status {
        CaseStatus::Unchanged => Color::White,
        CaseStatus::Added => Color::Cyan,
        CaseStatus::Removed => Color::Red,
        CaseStatus::Changed => Color::Yellow,
        CaseStatus::Improved => Color::Green,
    }
}

fn write_stdout(contents: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(contents.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn write_json_stdout<T: Serialize>(value: &T) -> Result<()> {
    let mut stdout = io::stdout().lock();
    write_json_writer(&mut stdout, value)?;
    stdout.flush()?;
    Ok(())
}

fn is_stdio_path(path: &Path) -> bool {
    path == Path::new("-")
}

fn display_path(path: &Path) -> String {
    if is_stdio_path(path) {
        "stdin".to_string()
    } else {
        viscompile::types::escape_untrusted_text(&path.to_string_lossy())
    }
}
