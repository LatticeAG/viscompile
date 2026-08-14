use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{self, ExitCode};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use viscompile::canonical::{
    load_snapshot_reader_with_limit, load_snapshot_with_limit, load_transcript_reader_with_limit,
    load_transcript_with_limit, snapshot_digest, write_json_file, write_json_writer,
    write_text_file,
};
use viscompile::compare::compare_snapshots;
use viscompile::diff::diff_snapshots;
use viscompile::format::{
    diff_text_lines, format_comparison_html, format_comparison_text, format_diff_html, DiffTextKind,
};
use viscompile::graph::format_diff_dot;
use viscompile::types::{DiffReport, Result, VisCompileError};

#[derive(Debug, Parser)]
#[command(
    name = "lattice",
    version,
    about = "VisCompile by LatticeAG - behavioral diff for AI agents"
)]
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
    /// Maximum raw input bytes per document. `0` disables the cap.
    #[arg(long, global = true, default_value = MAX_BYTES_DEFAULT)]
    max_bytes: u64,
    #[command(subcommand)]
    command: Commands,
}

const MAX_BYTES_DEFAULT: &str = "67108864";

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
        /// Write `sha256:<hex>` of the snapshot to this path, or `-` for stdout.
        #[arg(long)]
        digest_out: Option<PathBuf>,
    },
    /// Compatibility alias for `compile`.
    Snapshot {
        /// Path to a versioned transcript JSON file, or `-` for stdin.
        #[arg(short, long)]
        input: PathBuf,
        /// Path for the canonical snapshot JSON file, or `-` for stdout.
        #[arg(short, long)]
        out: PathBuf,
        /// Write `sha256:<hex>` of the snapshot to this path, or `-` for stdout.
        #[arg(long)]
        digest_out: Option<PathBuf>,
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
        /// Report selector. `summary` is the v1 diff document; `comparison` adds evidence.
        #[arg(long, value_enum, default_value_t = ReportKind::Summary)]
        report: ReportKind,
        /// Exit with status 2 when the diff contains regressions.
        #[arg(long)]
        fail_on_regression: bool,
        /// Require the loaded baseline snapshot digest to match `sha256:<hex>`.
        #[arg(long)]
        require_baseline_digest: Option<String>,
        /// Require the loaded target snapshot digest to match `sha256:<hex>`.
        #[arg(long)]
        require_target_digest: Option<String>,
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
        /// Require the loaded baseline snapshot digest to match `sha256:<hex>`.
        #[arg(long)]
        require_baseline_digest: Option<String>,
        /// Require the loaded target snapshot digest to match `sha256:<hex>`.
        #[arg(long)]
        require_target_digest: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DiffFormat {
    Text,
    Json,
    Html,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ReportKind {
    #[default]
    Summary,
    Comparison,
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            if err.exit_code() == 0 {
                process::exit(0);
            }
            process::exit(1);
        }
    };
    let verbosity = Verbosity::from_flags(cli.verbose, cli.quiet);

    match cli.command {
        Commands::Compile {
            input,
            out,
            digest_out,
        }
        | Commands::Snapshot {
            input,
            out,
            digest_out,
        } => {
            compile(
                &input,
                &out,
                digest_out.as_deref(),
                verbosity,
                cli.max_bytes,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Diff {
            baseline,
            target,
            out,
            format,
            color,
            report,
            fail_on_regression,
            require_baseline_digest,
            require_target_digest,
        } => {
            let loaded = load_diff(
                &baseline,
                &target,
                verbosity,
                cli.max_bytes,
                require_baseline_digest.as_deref(),
                require_target_digest.as_deref(),
            )?;
            write_diff_report(&loaded, out.as_deref(), format, report, color, verbosity)?;
            exit_for_regressions(loaded.report.summary.regressions, fail_on_regression)
        }
        Commands::Graph {
            baseline,
            target,
            out,
            fail_on_regression,
            require_baseline_digest,
            require_target_digest,
        } => {
            let loaded = load_diff(
                &baseline,
                &target,
                verbosity,
                cli.max_bytes,
                require_baseline_digest.as_deref(),
                require_target_digest.as_deref(),
            )?;
            write_graph(&loaded.report, out.as_deref(), verbosity)?;
            exit_for_regressions(loaded.report.summary.regressions, fail_on_regression)
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

fn compile(
    input: &Path,
    out: &Path,
    digest_out: Option<&Path>,
    verbosity: Verbosity,
    max_bytes: u64,
) -> Result<()> {
    verbosity.detail(format!("Reading transcript from {}", display_path(input)));
    let snapshot = load_transcript_input(input, max_bytes)?;
    let digest = snapshot_digest(&snapshot)?;

    if let Some(digest_path) = digest_out {
        if is_stdio_path(digest_path) && is_stdio_path(out) {
            return Err(VisCompileError::validation_code(
                "VC1304",
                "cannot write snapshot JSON and digest to the same stdout",
            ));
        }
        if same_output_path(digest_path, out) {
            return Err(VisCompileError::validation_code(
                "VC1303",
                "--digest-out must be a different path than --out",
            ));
        }
    }

    if is_stdio_path(out) {
        write_json_stdout(&snapshot)?;
        verbosity.status("Canonical snapshot written to stdout");
    } else {
        write_json_file(out, &snapshot)?;
        verbosity.status(format!("Canonical snapshot saved to {}", display_path(out)));
    }

    if let Some(digest_path) = digest_out {
        let line = format!("{digest}\n");
        if is_stdio_path(digest_path) {
            write_stdout(&line)?;
            verbosity.status("Digest written to stdout");
        } else {
            write_text_file(digest_path, &line)?;
            verbosity.status(format!("Digest saved to {}", display_path(digest_path)));
        }
    }

    verbosity.detail(format!("snapshot digest {digest}"));
    Ok(())
}

struct LoadedDiff {
    report: viscompile::types::DiffReport,
    baseline: viscompile::types::CanonicalSnapshot,
    target: viscompile::types::CanonicalSnapshot,
}

fn load_diff(
    baseline: &Path,
    target: &Path,
    verbosity: Verbosity,
    max_bytes: u64,
    require_baseline_digest: Option<&str>,
    require_target_digest: Option<&str>,
) -> Result<LoadedDiff> {
    let required_baseline = require_baseline_digest.map(parse_digest_flag).transpose()?;
    let required_target = require_target_digest.map(parse_digest_flag).transpose()?;
    if is_stdio_path(baseline) && is_stdio_path(target) {
        return Err(VisCompileError::validation_code(
            "VC1014",
            "only one of `--baseline` and `--target` may be `-` because stdin can be read once",
        ));
    }

    verbosity.detail(format!("Reading baseline from {}", display_path(baseline)));
    let baseline_snapshot = load_input(baseline, max_bytes)?;
    if let Some(expected) = required_baseline.as_deref() {
        check_required_digest("baseline", &baseline_snapshot, expected)?;
    }
    verbosity.detail(format!("Reading target from {}", display_path(target)));
    let target_snapshot = load_input(target, max_bytes)?;
    if let Some(expected) = required_target.as_deref() {
        check_required_digest("target", &target_snapshot, expected)?;
    }
    let report = diff_snapshots(&baseline_snapshot, &target_snapshot);
    verbosity.detail(format!(
        "Diff complete: {} cases, {} regressions, {} improvements",
        report.cases.len(),
        report.summary.regressions,
        report.summary.improvements
    ));
    Ok(LoadedDiff {
        report,
        baseline: baseline_snapshot,
        target: target_snapshot,
    })
}

fn load_input(path: &Path, max_bytes: u64) -> Result<viscompile::types::CanonicalSnapshot> {
    if is_stdio_path(path) {
        let stdin = io::stdin();
        load_snapshot_reader_with_limit(stdin.lock(), max_bytes)
    } else {
        load_snapshot_with_limit(path, max_bytes)
    }
}

fn load_transcript_input(
    path: &Path,
    max_bytes: u64,
) -> Result<viscompile::types::CanonicalSnapshot> {
    if is_stdio_path(path) {
        let stdin = io::stdin();
        load_transcript_reader_with_limit(stdin.lock(), max_bytes)
    } else {
        load_transcript_with_limit(path, max_bytes)
    }
}

fn write_diff_report(
    loaded: &LoadedDiff,
    output_path: Option<&Path>,
    format: DiffFormat,
    report_kind: ReportKind,
    color: ColorMode,
    verbosity: Verbosity,
) -> Result<()> {
    let output_to_stdout = output_path.is_some_and(is_stdio_path);
    if output_to_stdout && !matches!(format, DiffFormat::Json) {
        return Err(VisCompileError::validation_code(
            "VC1015",
            "`diff --out -` requires `--format json` so stdout remains a single document",
        ));
    }

    match report_kind {
        ReportKind::Summary => {
            if let Some(path) = output_path.filter(|path| !is_stdio_path(path)) {
                write_json_file(path, &loaded.report)?;
                verbosity.status(format!("Diff report saved to {}", display_path(path)));
            }
            match format {
                DiffFormat::Text => write_colored_diff_text(&loaded.report, color),
                DiffFormat::Json => write_json_stdout(&loaded.report),
                DiffFormat::Html => write_stdout(&format_diff_html(&loaded.report)),
            }
        }
        ReportKind::Comparison => {
            let comparison = compare_snapshots(&loaded.baseline, &loaded.target)?;
            if let Some(path) = output_path.filter(|path| !is_stdio_path(path)) {
                write_json_file(path, &comparison)?;
                verbosity.status(format!("Comparison report saved to {}", display_path(path)));
            }
            match format {
                DiffFormat::Text => write_stdout(&format_comparison_text(&comparison)),
                DiffFormat::Json => write_json_stdout(&comparison),
                DiffFormat::Html => write_stdout(&format_comparison_html(&comparison)),
            }
        }
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
    for line in diff_text_lines(report) {
        match line.kind {
            DiffTextKind::Heading => {
                write_styled(&mut stdout, color_spec(Color::Cyan, true), &line.text)?;
            }
            DiffTextKind::SummaryOk => {
                write_styled(&mut stdout, color_spec(Color::Green, true), &line.text)?;
            }
            DiffTextKind::SummaryRegressed => {
                write_styled(&mut stdout, color_spec(Color::Red, true), &line.text)?;
            }
            DiffTextKind::Body => {
                stdout.write_all(line.text.as_bytes())?;
            }
            DiffTextKind::StatusUnchanged => {
                write_styled(&mut stdout, color_spec(Color::White, false), &line.text)?;
            }
            DiffTextKind::StatusAdded => {
                write_styled(&mut stdout, color_spec(Color::Cyan, false), &line.text)?;
            }
            DiffTextKind::StatusRemoved => {
                write_styled(&mut stdout, color_spec(Color::Red, true), &line.text)?;
            }
            DiffTextKind::StatusChanged => {
                write_styled(&mut stdout, color_spec(Color::Yellow, true), &line.text)?;
            }
            DiffTextKind::StatusImproved => {
                write_styled(&mut stdout, color_spec(Color::Green, false), &line.text)?;
            }
        }
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

fn same_output_path(left: &Path, right: &Path) -> bool {
    if is_stdio_path(left) || is_stdio_path(right) {
        return is_stdio_path(left) && is_stdio_path(right);
    }
    if left == right {
        return true;
    }
    match (std::path::absolute(left), std::path::absolute(right)) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

fn parse_digest_flag(value: &str) -> Result<String> {
    if is_valid_snapshot_digest(value) {
        Ok(value.to_string())
    } else {
        Err(VisCompileError::validation_code(
            "VC1302",
            "digest must be sha256: followed by 64 lowercase hex characters",
        ))
    }
}

fn is_valid_snapshot_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_required_digest(
    side: &str,
    snapshot: &viscompile::types::CanonicalSnapshot,
    expected: &str,
) -> Result<()> {
    let actual = snapshot_digest(snapshot)?;
    if actual == expected {
        Ok(())
    } else {
        Err(VisCompileError::validation_code(
            "VC1301",
            format!("{side} snapshot digest {actual} does not match {expected}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn max_bytes_default_value_is_64_mib() {
        assert_eq!(super::MAX_BYTES_DEFAULT, "67108864");
        assert_eq!(
            super::MAX_BYTES_DEFAULT.parse::<u64>().expect("u64"),
            67_108_864
        );
    }
}
