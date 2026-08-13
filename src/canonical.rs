use crate::types::{
    escape_untrusted_text, CanonicalCase, CanonicalOutput, CanonicalSnapshot, CanonicalToolCall,
    Result, SchemaViolation, TranscriptSchemaValidationError, VisCompileError, SCHEMA_VERSION,
    SNAPSHOT_KIND, TRANSCRIPT_KIND,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// Keep malformed large transcripts from turning diagnostics into an unbounded allocation.
const MAX_SCHEMA_VIOLATIONS: usize = 100;

pub fn compile_path(input_path: &Path, output_path: &Path) -> Result<CanonicalSnapshot> {
    let snapshot = load_transcript(input_path)?;
    write_json_file(output_path, &snapshot)?;
    Ok(snapshot)
}

/// Load and canonicalize a raw transcript. Snapshots are intentionally rejected here because
/// compilation is a one-way conversion; callers that accept either format should use
/// [`load_snapshot`].
pub fn load_transcript(path: &Path) -> Result<CanonicalSnapshot> {
    let file = File::open(path)?;
    load_transcript_reader(BufReader::new(file))
}

/// Parse and canonicalize a raw transcript from a byte stream.
pub fn load_transcript_reader<R: Read>(reader: R) -> Result<CanonicalSnapshot> {
    let value: Value = serde_json::from_reader(reader)?;
    canonicalize_transcript(value)
}

pub fn load_snapshot(path: &Path) -> Result<CanonicalSnapshot> {
    let file = File::open(path)?;
    load_snapshot_reader(BufReader::new(file))
}

/// Parse a raw transcript or canonical snapshot from any byte stream and normalize it.
///
/// This is intentionally reader-based so CLI callers can use stdin without buffering an
/// additional UTF-8 string. Invalid UTF-8 and other malformed JSON are reported as a regular
/// `VisCompileError::Json` rather than panicking.
pub fn load_snapshot_reader<R: Read>(reader: R) -> Result<CanonicalSnapshot> {
    let value: Value = serde_json::from_reader(reader)?;
    canonicalize_document(value)
}

pub fn canonicalize_document(value: Value) -> Result<CanonicalSnapshot> {
    let kind = value
        .as_object()
        .ok_or_else(|| VisCompileError::validation("document must be a JSON object"))
        .and_then(|object| required_string(object, "kind", "document"))?;

    match kind.as_str() {
        TRANSCRIPT_KIND => canonicalize_transcript(value),
        SNAPSHOT_KIND => load_canonical_snapshot(value),
        _ => Err(VisCompileError::validation(format!(
            "document kind must be `{TRANSCRIPT_KIND}` or `{SNAPSHOT_KIND}`, found `{}`",
            escape_untrusted_text(&kind)
        ))),
    }
}

/// Canonicalize a raw transcript without accepting an already-canonical snapshot.
pub fn canonicalize_transcript(value: Value) -> Result<CanonicalSnapshot> {
    let object = value
        .as_object()
        .ok_or_else(|| VisCompileError::validation("document must be a JSON object"))?;
    let kind = required_string(object, "kind", "document")?;
    if kind != TRANSCRIPT_KIND {
        return Err(VisCompileError::validation(format!(
            "compile accepts only a raw transcript with kind `{TRANSCRIPT_KIND}`; snapshots are accepted only by `diff`"
        )));
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
    let validator = transcript_schema_validator()?;
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
    static VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
        OnceLock::new();

    VALIDATOR
        .get_or_init(|| {
            let schema: Value =
                serde_json::from_str(include_str!("../schemas/transcript.schema.json"))
                    .map_err(|error| error.to_string())?;
            jsonschema::draft202012::new(&schema).map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| VisCompileError::schema_definition(error.clone()))
}

fn compile_raw_transcript(document: &Map<String, Value>) -> Result<CanonicalSnapshot> {
    validate_schema_version(document, "transcript")?;
    let cases = required_array(document, "cases", "transcript")?;
    if cases.is_empty() {
        return Err(VisCompileError::validation(
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
    let object = raw_case
        .as_object()
        .ok_or_else(|| VisCompileError::validation(format!("{context} must be a JSON object")))?;
    let id = required_string(object, "id", &context)?;
    let display_id = escape_untrusted_text(&id);
    if !identifiers.insert(id.clone()) {
        return Err(VisCompileError::validation(format!(
            "duplicate case id `{display_id}`"
        )));
    }

    let input = canonicalize_value(required_value(object, "input", &context)?.clone())?;
    let events = required_array(object, "events", &context)?;
    if events.is_empty() {
        return Err(VisCompileError::validation(format!(
            "case `{display_id}` must contain at least one event"
        )));
    }

    let mut tool_calls = Vec::new();
    let mut output = CanonicalOutput::Absent;
    let mut error = None;

    for (event_index, event) in events.iter().enumerate() {
        let event_context = format!("case `{display_id}` event at index {event_index}");
        let event_object = event.as_object().ok_or_else(|| {
            VisCompileError::validation(format!("{event_context} must be a JSON object"))
        })?;
        let event_type = required_string(event_object, "type", &event_context)?;
        let is_last = event_index + 1 == events.len();

        match event_type.as_str() {
            "tool_call" => {
                if is_last {
                    return Err(VisCompileError::validation(format!(
                        "case `{display_id}` must end with a `final` or `error` event"
                    )));
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
                    return Err(VisCompileError::validation(format!(
                        "terminal `final` event for case `{display_id}` must be last"
                    )));
                }
                output = CanonicalOutput::Value(canonicalize_value(
                    required_value(event_object, "output", &event_context)?.clone(),
                )?);
            }
            "error" => {
                if !is_last {
                    return Err(VisCompileError::validation(format!(
                        "terminal `error` event for case `{display_id}` must be last"
                    )));
                }
                error = Some(normalize_nonempty_string(
                    required_string(event_object, "message", &event_context)?,
                    "error message",
                )?);
            }
            _ => {
                return Err(VisCompileError::validation(format!(
                    "{event_context} has unsupported event type `{}`; expected `tool_call`, `final`, or `error`",
                    escape_untrusted_text(&event_type)
                )));
            }
        }
    }

    if output.is_absent() && error.is_none() {
        return Err(VisCompileError::validation(format!(
            "case `{display_id}` must end with a `final` or `error` event"
        )));
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
    let snapshot: CanonicalSnapshot = serde_json::from_value(value).map_err(|error| {
        VisCompileError::validation(format!(
            "invalid canonical snapshot: {}",
            escape_untrusted_text(&error.to_string())
        ))
    })?;
    if snapshot.kind != SNAPSHOT_KIND {
        return Err(VisCompileError::validation(format!(
            "snapshot kind must be `{SNAPSHOT_KIND}`"
        )));
    }
    if snapshot.schema_version != SCHEMA_VERSION {
        return Err(VisCompileError::validation(format!(
            "snapshot schema_version must be {SCHEMA_VERSION}"
        )));
    }
    normalize_snapshot(snapshot)
}

fn normalize_snapshot(mut snapshot: CanonicalSnapshot) -> Result<CanonicalSnapshot> {
    if snapshot.cases.is_empty() {
        return Err(VisCompileError::validation(
            "snapshot `cases` must contain at least one case",
        ));
    }

    let mut identifiers = BTreeSet::new();
    for case in &mut snapshot.cases {
        case.id = normalize_nonempty_string(case.id.clone(), "case id")?;
        let display_id = escape_untrusted_text(&case.id);
        if !identifiers.insert(case.id.clone()) {
            return Err(VisCompileError::validation(format!(
                "duplicate case id `{display_id}`"
            )));
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
                return Err(VisCompileError::validation(format!(
                    "case `{display_id}` cannot contain both `output` and `error`"
                )));
            }
            (false, false) => {
                return Err(VisCompileError::validation(format!(
                    "case `{display_id}` must contain `output` or `error`"
                )));
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
            VisCompileError::validation(format!(
                "{document_name} `schema_version` must be an unsigned integer"
            ))
        })?;
    if version != u64::from(SCHEMA_VERSION) {
        return Err(VisCompileError::validation(format!(
            "{document_name} schema_version must be {SCHEMA_VERSION}"
        )));
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
        .ok_or_else(|| VisCompileError::validation(format!("{context} `{field}` must be an array")))
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Value> {
    object.get(field).ok_or_else(|| {
        VisCompileError::validation(format!("{context} is missing required field `{field}`"))
    })
}

fn required_string(object: &Map<String, Value>, field: &str, context: &str) -> Result<String> {
    let value = required_value(object, field, context)?
        .as_str()
        .ok_or_else(|| {
            VisCompileError::validation(format!("{context} `{field}` must be a string"))
        })?;
    normalize_nonempty_string(value.to_string(), field)
}

fn normalize_nonempty_string(value: String, field_name: &str) -> Result<String> {
    let normalized = normalize_string(value);
    if normalized.trim().is_empty() {
        return Err(VisCompileError::validation(format!(
            "{} must not be blank",
            escape_untrusted_text(field_name)
        )));
    }
    Ok(normalized)
}

fn canonicalize_arguments(value: Value, context: &str) -> Result<Value> {
    if let Value::String(text) = &value {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let parsed: Value = serde_json::from_str(text).map_err(|error| {
                VisCompileError::validation(format!(
                    "{context} `arguments` contains invalid JSON: {}",
                    escape_untrusted_text(&error.to_string())
                ))
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
                    return Err(VisCompileError::validation(format!(
                        "JSON object keys collide after CRLF normalization: `{}`",
                        normalized_key.escape_default()
                    )));
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
    let file_name = path
        .file_name()
        .ok_or_else(|| VisCompileError::validation("output path must include a file name"))?;
    let file_name = file_name.to_string_lossy();

    let (temporary_path, mut temporary_file) = create_temporary_file(parent, &file_name)?;
    let write_result = (|| -> Result<()> {
        write_contents(&mut temporary_file)?;
        temporary_file.sync_all()?;
        Ok(())
    })();
    drop(temporary_file);

    let write_result = write_result.and_then(|()| {
        fs::rename(&temporary_path, path)?;
        Ok(())
    });

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
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
    Err(VisCompileError::validation(format!(
        "could not create a temporary output file next to `{}`",
        escape_untrusted_text(file_name)
    )))
}
