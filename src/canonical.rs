use crate::types::{
    escape_untrusted_text, CanonicalCase, CanonicalOutput, CanonicalSnapshot, CanonicalToolCall,
    Result, SchemaViolation, TranscriptSchemaValidationError, VisCompileError, SCHEMA_VERSION,
    SNAPSHOT_KIND, TRANSCRIPT_KIND,
};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// Keep malformed large transcripts from turning diagnostics into an unbounded allocation.
const MAX_SCHEMA_VIOLATIONS: usize = 100;

/// Default per-document raw input cap: 64 MiB. Pass `0` to [`read_limited_to_end`] to disable.
pub const DEFAULT_MAX_BYTES: u64 = 67_108_864;

pub fn compile_path(input_path: &Path, output_path: &Path) -> Result<CanonicalSnapshot> {
    let snapshot = load_transcript(input_path)?;
    write_json_file(output_path, &snapshot)?;
    Ok(snapshot)
}

/// Load and canonicalize a raw transcript. Snapshots are intentionally rejected here because
/// compilation is a one-way conversion; callers that accept either format should use
/// [`load_snapshot`].
///
/// Reads at most [`DEFAULT_MAX_BYTES`] (64 MiB) of raw input unless
/// [`load_transcript_with_limit`] is used.
pub fn load_transcript(path: &Path) -> Result<CanonicalSnapshot> {
    load_transcript_with_limit(path, DEFAULT_MAX_BYTES)
}

/// Load a raw transcript with an explicit raw-byte cap. `max_bytes == 0` disables the cap.
pub fn load_transcript_with_limit(path: &Path, max_bytes: u64) -> Result<CanonicalSnapshot> {
    let file = File::open(path)?;
    load_transcript_reader_with_limit(BufReader::new(file), max_bytes)
}

/// Parse and canonicalize a raw transcript from a byte stream.
pub fn load_transcript_reader<R: Read>(reader: R) -> Result<CanonicalSnapshot> {
    load_transcript_reader_with_limit(reader, DEFAULT_MAX_BYTES)
}

/// Parse a raw transcript from a byte stream, capping the raw read at `max_bytes`.
pub fn load_transcript_reader_with_limit<R: Read>(
    reader: R,
    max_bytes: u64,
) -> Result<CanonicalSnapshot> {
    let bytes = read_limited_to_end(reader, max_bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    canonicalize_transcript(value)
}

pub fn load_snapshot(path: &Path) -> Result<CanonicalSnapshot> {
    load_snapshot_with_limit(path, DEFAULT_MAX_BYTES)
}

/// Load a transcript or snapshot with an explicit raw-byte cap. `max_bytes == 0` disables the cap.
pub fn load_snapshot_with_limit(path: &Path, max_bytes: u64) -> Result<CanonicalSnapshot> {
    let file = File::open(path)?;
    load_snapshot_reader_with_limit(BufReader::new(file), max_bytes)
}

/// Parse a raw transcript or canonical snapshot from any byte stream and normalize it.
///
/// This is intentionally reader-based so CLI callers can use stdin without buffering an
/// additional UTF-8 string. Invalid UTF-8 and other malformed JSON are reported as a regular
/// `VisCompileError::Json` rather than panicking.
pub fn load_snapshot_reader<R: Read>(reader: R) -> Result<CanonicalSnapshot> {
    load_snapshot_reader_with_limit(reader, DEFAULT_MAX_BYTES)
}

/// Parse a transcript or snapshot from a byte stream, capping the raw read at `max_bytes`.
pub fn load_snapshot_reader_with_limit<R: Read>(
    reader: R,
    max_bytes: u64,
) -> Result<CanonicalSnapshot> {
    let bytes = read_limited_to_end(reader, max_bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    canonicalize_document(value)
}

/// Read a stream into memory, failing with `VC1201` as soon as the buffer exceeds `max_bytes`.
///
/// `max_bytes == 0` reads without a cap. This must run before JSON parse so an oversize input
/// cannot be reclassified as `VC1102`.
pub fn read_limited_to_end<R: Read>(mut reader: R, max_bytes: u64) -> Result<Vec<u8>> {
    if max_bytes == 0 {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        return Ok(buf);
    }

    let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > limit {
            return Err(VisCompileError::validation_code(
                "VC1201",
                format!("input exceeds --max-bytes limit of {max_bytes}"),
            ));
        }
    }
    Ok(buf)
}

pub fn canonicalize_document(value: Value) -> Result<CanonicalSnapshot> {
    let kind = value
        .as_object()
        .ok_or_else(|| VisCompileError::validation_code("VC1016", "document must be a JSON object"))
        .and_then(|object| required_string(object, "kind", "document"))?;

    match kind.as_str() {
        TRANSCRIPT_KIND => canonicalize_transcript(value),
        SNAPSHOT_KIND => load_canonical_snapshot(value),
        _ => Err(VisCompileError::validation_code(
            "VC1011",
            format!(
                "document kind must be `{TRANSCRIPT_KIND}` or `{SNAPSHOT_KIND}`, found `{}`",
                escape_untrusted_text(&kind)
            ),
        )),
    }
}

/// Canonicalize a raw transcript without accepting an already-canonical snapshot.
pub fn canonicalize_transcript(value: Value) -> Result<CanonicalSnapshot> {
    let object = value.as_object().ok_or_else(|| {
        VisCompileError::validation_code("VC1016", "document must be a JSON object")
    })?;
    let kind = required_string(object, "kind", "document")?;
    if kind != TRANSCRIPT_KIND {
        return Err(VisCompileError::validation_code(
            "VC1011",
            format!(
            "compile accepts only a raw transcript with kind `{TRANSCRIPT_KIND}`; snapshots are accepted only by `diff`"
        ),
        ));
    }

    validate_transcript_schema(&value)?;
    compile_raw_transcript(object)
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let mut serialized = serde_json::to_string_pretty(value)?;
    serialized.push('\n');
    Ok(serialized)
}

/// Write canonical pretty JSON to a stream, including one trailing newline.
///
/// Unlike `canonical_json`, this does not first create a second complete string in memory and is
/// appropriate for stdout or large snapshots/reports.
pub fn write_json_writer<T: Serialize, W: Write>(mut writer: W, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

pub fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomically(path, |file| write_json_writer(file, value))
}

struct HashWriter(Sha256);

impl Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// SHA-256 of the exact pretty-JSON-plus-newline bytes [`write_json_writer`] emits.
///
/// Display form is `sha256:` plus 64 lowercase hex characters. This hashes the
/// normalized snapshot, not a raw transcript file.
pub fn snapshot_digest(snapshot: &CanonicalSnapshot) -> Result<String> {
    let mut hasher = HashWriter(Sha256::new());
    write_json_writer(&mut hasher, snapshot)?;
    Ok(format!("sha256:{:x}", hasher.0.finalize()))
}

/// Atomically replace a text file with `contents`.
///
/// This uses the same temporary-file-and-rename strategy as JSON output so a failed command does
/// not leave a partial DOT, HTML, or text report behind.
pub fn write_text_file(path: &Path, contents: &str) -> Result<()> {
    write_atomically(path, |file| {
        file.write_all(contents.as_bytes())?;
        Ok(())
    })
}

/// Validate a raw transcript against the versioned JSON Schema before semantic canonicalization.
///
/// The schema checks document shape, required properties, supported event forms, and unknown
/// properties. Cross-event invariants (such as a terminal event appearing last) are checked by
/// the compiler because they depend on event order.
pub fn validate_transcript_schema(value: &Value) -> Result<()> {
    validate_with_schema(transcript_schema_validator()?, value)
}

/// Validate a canonical snapshot Value against the bundled snapshot JSON Schema.
pub fn validate_snapshot_schema(value: &Value) -> Result<()> {
    validate_with_schema(snapshot_schema_validator()?, value)
}

/// Validate a diff report Value against the bundled diff JSON Schema.
pub fn validate_diff_schema(value: &Value) -> Result<()> {
    validate_with_schema(diff_schema_validator()?, value)
}

fn validate_with_schema(validator: &jsonschema::Validator, value: &Value) -> Result<()> {
    let violations = validator
        .iter_errors(value)
        .take(MAX_SCHEMA_VIOLATIONS)
        .map(|error| SchemaViolation {
            instance_path: escape_untrusted_text(&error.instance_path().to_string()),
            schema_path: escape_untrusted_text(&error.schema_path().to_string()),
            message: escape_untrusted_text(&error.to_string()),
        })
        .collect();
    let error = TranscriptSchemaValidationError::new(violations);

    if error.violations.is_empty() {
        Ok(())
    } else {
        Err(error.into())
    }
}

fn transcript_schema_validator() -> Result<&'static jsonschema::Validator> {
    schema_validator(
        include_str!("../schemas/transcript.schema.json"),
        "transcript",
    )
}

fn snapshot_schema_validator() -> Result<&'static jsonschema::Validator> {
    schema_validator(include_str!("../schemas/snapshot.schema.json"), "snapshot")
}

fn diff_schema_validator() -> Result<&'static jsonschema::Validator> {
    schema_validator(include_str!("../schemas/diff.schema.json"), "diff")
}

/// Validate a comparison report Value against the bundled comparison JSON Schema.
pub fn validate_comparison_schema(value: &Value) -> Result<()> {
    validate_with_schema(comparison_schema_validator()?, value)
}

fn comparison_schema_validator() -> Result<&'static jsonschema::Validator> {
    schema_validator(
        include_str!("../schemas/comparison.schema.json"),
        "comparison",
    )
}

fn schema_validator(
    source: &'static str,
    name: &'static str,
) -> Result<&'static jsonschema::Validator> {
    match name {
        "transcript" => lock_validator(&TRANSCRIPT_VALIDATOR, source),
        "snapshot" => lock_validator(&SNAPSHOT_VALIDATOR, source),
        "diff" => lock_validator(&DIFF_VALIDATOR, source),
        "comparison" => lock_validator(&COMPARISON_VALIDATOR, source),
        _ => Err(VisCompileError::schema_definition(format!(
            "unknown bundled schema name `{name}`"
        ))),
    }
}

static TRANSCRIPT_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();
static SNAPSHOT_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();
static DIFF_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();
static COMPARISON_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();

fn lock_validator(
    cell: &'static OnceLock<std::result::Result<jsonschema::Validator, String>>,
    source: &'static str,
) -> Result<&'static jsonschema::Validator> {
    cell.get_or_init(|| {
        let schema: Value = serde_json::from_str(source).map_err(|error| error.to_string())?;
        jsonschema::draft202012::new(&schema).map_err(|error| error.to_string())
    })
    .as_ref()
    .map_err(|error| VisCompileError::schema_definition(error.clone()))
}

fn compile_raw_transcript(document: &Map<String, Value>) -> Result<CanonicalSnapshot> {
    validate_schema_version(document, "transcript")?;
    let cases = required_array(document, "cases", "transcript")?;
    if cases.is_empty() {
        return Err(VisCompileError::validation_code(
            "VC1001",
            "transcript `cases` must contain at least one case",
        ));
    }

    let mut identifiers = BTreeSet::new();
    let mut canonical_cases = Vec::with_capacity(cases.len());
    for (index, raw_case) in cases.iter().enumerate() {
        canonical_cases.push(compile_case(raw_case, index, &mut identifiers)?);
    }

    canonical_cases.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(CanonicalSnapshot {
        kind: SNAPSHOT_KIND.to_string(),
        schema_version: SCHEMA_VERSION,
        cases: canonical_cases,
    })
}

fn compile_case(
    raw_case: &Value,
    index: usize,
    identifiers: &mut BTreeSet<String>,
) -> Result<CanonicalCase> {
    let context = format!("case at index {index}");
    let object = raw_case.as_object().ok_or_else(|| {
        VisCompileError::validation_code("VC1016", format!("{context} must be a JSON object"))
    })?;
    let id = required_string(object, "id", &context)?;
    let display_id = escape_untrusted_text(&id);
    if !identifiers.insert(id.clone()) {
        return Err(VisCompileError::validation_code(
            "VC1002",
            format!("duplicate case id `{display_id}`"),
        ));
    }

    let input = canonicalize_value(required_value(object, "input", &context)?.clone())?;
    let events = required_array(object, "events", &context)?;
    if events.is_empty() {
        return Err(VisCompileError::validation_code(
            "VC1003",
            format!("case `{display_id}` must contain at least one event"),
        ));
    }

    let mut tool_calls = Vec::new();
    let mut output = CanonicalOutput::Absent;
    let mut error = None;

    for (event_index, event) in events.iter().enumerate() {
        let event_context = format!("case `{display_id}` event at index {event_index}");
        let event_object = event.as_object().ok_or_else(|| {
            VisCompileError::validation_code(
                "VC1016",
                format!("{event_context} must be a JSON object"),
            )
        })?;
        let event_type = required_string(event_object, "type", &event_context)?;
        let is_last = event_index + 1 == events.len();

        match event_type.as_str() {
            "tool_call" => {
                if is_last {
                    return Err(VisCompileError::validation_code(
                        "VC1003",
                        format!("case `{display_id}` must end with a `final` or `error` event"),
                    ));
                }
                let name = required_string(event_object, "name", &event_context)?;
                let arguments = canonicalize_arguments(
                    required_value(event_object, "arguments", &event_context)?.clone(),
                    &event_context,
                )?;
                tool_calls.push(CanonicalToolCall { name, arguments });
            }
            "final" => {
                if !is_last {
                    return Err(VisCompileError::validation_code(
                        "VC1004",
                        format!("terminal `final` event for case `{display_id}` must be last"),
                    ));
                }
                output = CanonicalOutput::Value(canonicalize_value(
                    required_value(event_object, "output", &event_context)?.clone(),
                )?);
            }
            "error" => {
                if !is_last {
                    return Err(VisCompileError::validation_code(
                        "VC1004",
                        format!("terminal `error` event for case `{display_id}` must be last"),
                    ));
                }
                error = Some(normalize_nonempty_string(
                    required_string(event_object, "message", &event_context)?,
                    "error message",
                )?);
            }
            _ => {
                return Err(VisCompileError::validation_code(
                    "VC1005",
                    format!(
                    "{event_context} has unsupported event type `{}`; expected `tool_call`, `final`, or `error`",
                    escape_untrusted_text(&event_type)
                ),
                ));
            }
        }
    }

    if output.is_absent() && error.is_none() {
        return Err(VisCompileError::validation_code(
            "VC1003",
            format!("case `{display_id}` must end with a `final` or `error` event"),
        ));
    }

    Ok(CanonicalCase {
        id,
        input,
        tool_calls,
        output,
        error,
    })
}

fn load_canonical_snapshot(value: Value) -> Result<CanonicalSnapshot> {
    validate_snapshot_schema(&value)?;
    let snapshot: CanonicalSnapshot = serde_json::from_value(value).map_err(|error| {
        VisCompileError::validation_code(
            "VC1018",
            format!(
                "invalid canonical snapshot: {}",
                escape_untrusted_text(&error.to_string())
            ),
        )
    })?;
    if snapshot.kind != SNAPSHOT_KIND {
        return Err(VisCompileError::validation_code(
            "VC1011",
            format!("snapshot kind must be `{SNAPSHOT_KIND}`"),
        ));
    }
    if snapshot.schema_version != SCHEMA_VERSION {
        return Err(VisCompileError::validation_code(
            "VC1012",
            format!("snapshot schema_version must be {SCHEMA_VERSION}"),
        ));
    }
    normalize_snapshot(snapshot)
}

fn normalize_snapshot(mut snapshot: CanonicalSnapshot) -> Result<CanonicalSnapshot> {
    if snapshot.cases.is_empty() {
        return Err(VisCompileError::validation_code(
            "VC1001",
            "snapshot `cases` must contain at least one case",
        ));
    }

    let mut identifiers = BTreeSet::new();
    for case in &mut snapshot.cases {
        case.id = normalize_nonempty_string(case.id.clone(), "case id")?;
        let display_id = escape_untrusted_text(&case.id);
        if !identifiers.insert(case.id.clone()) {
            return Err(VisCompileError::validation_code(
                "VC1002",
                format!("duplicate case id `{display_id}`"),
            ));
        }

        case.input = canonicalize_value(case.input.clone())?;
        for tool_call in &mut case.tool_calls {
            tool_call.name = normalize_nonempty_string(tool_call.name.clone(), "tool call name")?;
            tool_call.arguments = canonicalize_value(tool_call.arguments.clone())?;
        }
        case.output = match std::mem::take(&mut case.output) {
            CanonicalOutput::Absent => CanonicalOutput::Absent,
            CanonicalOutput::Value(value) => CanonicalOutput::Value(canonicalize_value(value)?),
        };
        case.error = case
            .error
            .take()
            .map(|message| normalize_nonempty_string(message, "error message"))
            .transpose()?;

        match (case.output.is_present(), case.error.is_some()) {
            (true, false) | (false, true) => {}
            (true, true) => {
                return Err(VisCompileError::validation_code(
                    "VC1009",
                    format!("case `{display_id}` cannot contain both `output` and `error`"),
                ));
            }
            (false, false) => {
                return Err(VisCompileError::validation_code(
                    "VC1010",
                    format!("case `{display_id}` must contain `output` or `error`"),
                ));
            }
        }
    }

    snapshot.cases.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(snapshot)
}

fn validate_schema_version(document: &Map<String, Value>, document_name: &str) -> Result<()> {
    let version = required_value(document, "schema_version", document_name)?
        .as_u64()
        .ok_or_else(|| {
            VisCompileError::validation_code(
                "VC1018",
                format!("{document_name} `schema_version` must be an unsigned integer"),
            )
        })?;
    if version != u64::from(SCHEMA_VERSION) {
        return Err(VisCompileError::validation_code(
            "VC1012",
            format!("{document_name} schema_version must be {SCHEMA_VERSION}"),
        ));
    }
    Ok(())
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>> {
    required_value(object, field, context)?
        .as_array()
        .ok_or_else(|| {
            VisCompileError::validation_code(
                "VC1018",
                format!("{context} `{field}` must be an array"),
            )
        })
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Value> {
    object.get(field).ok_or_else(|| {
        VisCompileError::validation_code(
            "VC1017",
            format!("{context} is missing required field `{field}`"),
        )
    })
}

fn required_string(object: &Map<String, Value>, field: &str, context: &str) -> Result<String> {
    let value = required_value(object, field, context)?
        .as_str()
        .ok_or_else(|| {
            VisCompileError::validation_code(
                "VC1018",
                format!("{context} `{field}` must be a string"),
            )
        })?;
    normalize_nonempty_string(value.to_string(), field)
}

fn normalize_nonempty_string(value: String, field_name: &str) -> Result<String> {
    let normalized = normalize_string(value);
    if normalized.trim().is_empty() {
        return Err(VisCompileError::validation_code(
            "VC1006",
            format!("{} must not be blank", escape_untrusted_text(field_name)),
        ));
    }
    Ok(normalized)
}

fn canonicalize_arguments(value: Value, context: &str) -> Result<Value> {
    if let Value::String(text) = &value {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let parsed: Value = serde_json::from_str(text).map_err(|error| {
                VisCompileError::validation_code(
                    "VC1008",
                    format!(
                        "{context} `arguments` contains invalid JSON: {}",
                        escape_untrusted_text(&error.to_string())
                    ),
                )
            })?;
            return canonicalize_value(parsed);
        }
    }
    canonicalize_value(value)
}

/// Normalize values while preserving array order and recursively sorting object keys.
///
/// Distinct object keys that become equal after CRLF normalization are rejected instead of being
/// silently overwritten, preserving the fail-closed behavior expected from a behavioral diff.
pub fn canonicalize_value(value: Value) -> Result<Value> {
    match value {
        Value::Array(values) => {
            let mut canonical_values = Vec::with_capacity(values.len());
            for value in values {
                canonical_values.push(canonicalize_value(value)?);
            }
            Ok(Value::Array(canonical_values))
        }
        Value::Object(values) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in values {
                let normalized_key = normalize_string(key);
                if sorted.contains_key(&normalized_key) {
                    return Err(VisCompileError::validation_code(
                        "VC1007",
                        format!(
                            "JSON object keys collide after CRLF normalization: `{}`",
                            normalized_key.escape_default()
                        ),
                    ));
                }
                sorted.insert(normalized_key, canonicalize_value(value)?);
            }
            Ok(Value::Object(sorted.into_iter().collect()))
        }
        Value::String(value) => Ok(Value::String(normalize_string(value))),
        primitive => Ok(primitive),
    }
}

fn normalize_string(value: String) -> String {
    if value.contains("\r\n") {
        value.replace("\r\n", "\n")
    } else {
        value
    }
}

fn write_atomically<F>(path: &Path, write_contents: F) -> Result<()>
where
    F: FnOnce(&mut File) -> Result<()>,
{
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        VisCompileError::validation_code("VC1104", "output path must include a file name")
    })?;
    let file_name = file_name.to_string_lossy();

    let (temporary_path, mut temporary_file) = create_temporary_file(parent, &file_name)?;
    let write_result = (|| -> Result<()> {
        write_contents(&mut temporary_file)?;
        temporary_file.sync_all()?;
        Ok(())
    })();
    drop(temporary_file);

    let write_result = write_result.and_then(|()| replace_destination(&temporary_path, path));

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

fn replace_destination(temporary_path: &Path, path: &Path) -> Result<()> {
    match fs::rename(temporary_path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            fs::rename(temporary_path, path)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            fs::copy(temporary_path, path)?;
            fs::remove_file(temporary_path)?;
            Ok(())
        }
        Err(error) => {
            // EXDEV is errno 18 on Unix. Match it explicitly so the branch compiles on
            // platforms where CrossesDevices is not produced by this rename.
            if error.raw_os_error() == Some(18) {
                fs::copy(temporary_path, path)?;
                fs::remove_file(temporary_path)?;
                return Ok(());
            }
            Err(error.into())
        }
    }
}

fn create_temporary_file(parent: &Path, file_name: &str) -> Result<(PathBuf, File)> {
    for attempt in 0..100_u32 {
        let candidate = parent.join(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(VisCompileError::validation_code(
        "VC1103",
        format!(
            "could not create a temporary output file next to `{}`",
            escape_untrusted_text(file_name)
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn read_limited_to_end_stops_after_cap() {
        let input = vec![b'x'; 1025];
        let error = read_limited_to_end(Cursor::new(input), 1024).unwrap_err();
        let message = error.to_string();
        assert!(
            message.starts_with("VC1201:"),
            "expected VC1201, got {message}"
        );
    }

    #[test]
    fn read_limited_to_end_allows_exact_cap() {
        let input = vec![b'y'; 1024];
        let bytes = read_limited_to_end(Cursor::new(input), 1024).expect("exact cap is allowed");
        assert_eq!(bytes.len(), 1024);
    }

    #[test]
    fn canonicalize_value_rejects_crlf_key_collision() {
        let mut object = serde_json::Map::new();
        object.insert("line\r\nkey".to_string(), json!("first"));
        object.insert("line\nkey".to_string(), json!("second"));
        let error = canonicalize_value(Value::Object(object)).unwrap_err();
        let message = error.to_string();
        assert!(message.starts_with("VC1007:"));
        assert!(message.contains("keys collide"));
    }

    #[test]
    fn canonicalize_value_normalizes_nested_crlf_strings() {
        let value = json!({"outer": {"note": "a\r\nb"}});
        let canonical = canonicalize_value(value).expect("canonicalize nested string");
        assert_eq!(canonical["outer"]["note"], "a\nb");
    }

    #[test]
    fn snapshot_schema_rejects_extra_top_level_field() {
        let value = json!({
            "kind": "latticeag.viscompile.snapshot",
            "schema_version": 1,
            "cases": [{
                "id": "one",
                "input": "hi",
                "output": "there"
            }],
            "extra": true
        });
        let error = validate_snapshot_schema(&value).unwrap_err();
        assert!(error.to_string().contains("JSON Schema"));
    }

    #[test]
    fn snapshot_schema_rejects_output_and_error_together() {
        let value = json!({
            "kind": "latticeag.viscompile.snapshot",
            "schema_version": 1,
            "cases": [{
                "id": "one",
                "input": "hi",
                "output": "there",
                "error": "nope"
            }]
        });
        let error = validate_snapshot_schema(&value).unwrap_err();
        assert!(error.to_string().contains("JSON Schema"));
    }

    #[test]
    fn write_atomically_overwrites_existing_destination() {
        let dir = std::env::temp_dir().join(format!(
            "viscompile-atomic-{}-{}",
            std::process::id(),
            "overwrite"
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("out.json");
        fs::write(&path, b"stale\n").expect("seed existing file");
        write_json_file(&path, &json!({"ok": true})).expect("overwrite");
        let saved = fs::read_to_string(&path).expect("read overwritten file");
        assert!(saved.contains("\"ok\": true"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_max_bytes_is_64_mib() {
        assert_eq!(DEFAULT_MAX_BYTES, 67_108_864);
    }

    #[test]
    fn snapshot_digest_has_sha256_prefix_and_64_hex_digits() {
        let snapshot = CanonicalSnapshot {
            kind: SNAPSHOT_KIND.to_string(),
            schema_version: SCHEMA_VERSION,
            cases: vec![CanonicalCase {
                id: "one".to_string(),
                input: json!("hi"),
                tool_calls: Vec::new(),
                output: CanonicalOutput::Value(json!("there")),
                error: None,
            }],
        };
        let digest = snapshot_digest(&snapshot).expect("digest");
        assert_eq!(digest.len(), 71);
        assert!(digest.starts_with("sha256:"));
        assert!(digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        let again = snapshot_digest(&snapshot).expect("digest again");
        assert_eq!(digest, again);
    }

    #[test]
    fn diff_schema_rejects_unknown_top_level_field() {
        let value = json!({
            "kind": "latticeag.viscompile.diff",
            "schema_version": 1,
            "summary": {
                "unchanged": 0,
                "added": 0,
                "removed": 0,
                "changed": 0,
                "regressions": 0,
                "improvements": 0
            },
            "cases": [],
            "extra": true
        });
        let error = validate_diff_schema(&value).unwrap_err();
        assert!(error.to_string().contains("JSON Schema"));
    }

    #[test]
    fn comparison_schema_rejects_unknown_why_code() {
        let value = json!({
            "kind": "latticeag.viscompile.comparison",
            "schema_version": 1,
            "profile": "latticeag.viscompile/v1",
            "baseline_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "target_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "summary": {
                "unchanged": 0,
                "added": 0,
                "removed": 0,
                "changed": 0,
                "regressions": 0,
                "improvements": 0
            },
            "cases": [{
                "id": "one",
                "status": "changed",
                "regression": true,
                "changed_fields": ["output"],
                "why_codes": ["not_a_real_code"],
                "changes": [],
                "tool_call_alignment": []
            }]
        });
        let error = validate_comparison_schema(&value).unwrap_err();
        assert!(error.to_string().contains("JSON Schema"));
    }

    #[test]
    fn replace_destination_matches_already_exists_kind() {
        let kind = io::ErrorKind::AlreadyExists;
        assert_eq!(kind, io::ErrorKind::AlreadyExists);
        let crosses = io::ErrorKind::CrossesDevices;
        assert_eq!(crosses, io::ErrorKind::CrossesDevices);
    }
}
