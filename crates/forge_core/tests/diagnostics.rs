use forge_core::{DiagnosticBuffer, DiagnosticLevel, DiagnosticRecord};

#[test]
fn diagnostic_buffer_retains_recent_records_and_counts_drops() {
    let mut buffer = DiagnosticBuffer::with_capacity(2);
    buffer.push(DiagnosticRecord::new(
        DiagnosticLevel::Info,
        "runtime.started",
        "runtime is ready",
    ));
    buffer.push(DiagnosticRecord::new(
        DiagnosticLevel::Warning,
        "runtime.frame_drop",
        "fixed-step work was dropped",
    ));
    buffer.push(DiagnosticRecord::new(
        DiagnosticLevel::Error,
        "runtime.script_failed",
        "script callback failed",
    ));

    assert_eq!(buffer.records().len(), 2);
    assert_eq!(buffer.records()[0].event, "runtime.frame_drop");
    assert_eq!(buffer.records()[1].event, "runtime.script_failed");
    assert_eq!(buffer.dropped_records(), 1);
}

#[test]
fn diagnostic_record_serializes_a_versioned_structured_event() {
    let record = DiagnosticRecord::new(
        DiagnosticLevel::Error,
        "editor.save_failed",
        "could not write the scene",
    )
    .with_field("path", "Scenes/Main.forgescene");

    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&record).expect("serialize record"))
            .expect("parse serialized record");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["level"], "error");
    assert_eq!(value["event"], "editor.save_failed");
    assert_eq!(value["message"], "could not write the scene");
    assert_eq!(value["fields"]["path"], "Scenes/Main.forgescene");
    assert!(value["timestamp_unix_ms"].as_u64().is_some());
}
