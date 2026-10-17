//! Bounded, schema-versioned records for editor and player diagnostics.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const CURRENT_DIAGNOSTIC_SCHEMA: u32 = 1;
pub const DEFAULT_DIAGNOSTIC_BUFFER_CAPACITY: usize = 256;
const MAX_EVENT_BYTES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_FIELD_COUNT: usize = 16;
const MAX_FIELD_NAME_BYTES: usize = 64;
const MAX_FIELD_VALUE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticRecord {
    pub schema_version: u32,
    pub timestamp_unix_ms: u64,
    pub level: DiagnosticLevel,
    pub event: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

impl DiagnosticRecord {
    pub fn new(level: DiagnosticLevel, event: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or_default();
        Self {
            schema_version: CURRENT_DIAGNOSTIC_SCHEMA,
            timestamp_unix_ms,
            level,
            event: truncate_utf8(event.as_ref(), MAX_EVENT_BYTES),
            message: truncate_utf8(message.as_ref(), MAX_MESSAGE_BYTES),
            fields: BTreeMap::new(),
        }
    }

    pub fn with_field(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let name = truncate_utf8(name.as_ref(), MAX_FIELD_NAME_BYTES);
        if name.is_empty() {
            return self;
        }
        let Some(existing) = self.fields.get_mut(&name) else {
            if self.fields.len() >= MAX_FIELD_COUNT {
                return self;
            }
            self.fields
                .insert(name, truncate_utf8(value.as_ref(), MAX_FIELD_VALUE_BYTES));
            return self;
        };
        *existing = truncate_utf8(value.as_ref(), MAX_FIELD_VALUE_BYTES);
        self
    }
}

#[derive(Clone, Debug)]
pub struct DiagnosticBuffer {
    capacity: usize,
    dropped_records: u64,
    records: VecDeque<DiagnosticRecord>,
}

impl DiagnosticBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            dropped_records: 0,
            records: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, record: DiagnosticRecord) {
        if self.records.len() == self.capacity {
            self.records.pop_front();
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
        self.records.push_back(record);
    }

    pub fn records(&self) -> &VecDeque<DiagnosticRecord> {
        &self.records
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.dropped_records = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }
}

impl Default for DiagnosticBuffer {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_DIAGNOSTIC_BUFFER_CAPACITY)
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_text_and_fields_are_bounded_at_utf8_boundaries() {
        let record = DiagnosticRecord::new(
            DiagnosticLevel::Error,
            format!("{}界", "e".repeat(MAX_EVENT_BYTES - 1)),
            "界".repeat(MAX_MESSAGE_BYTES),
        )
        .with_field("context", "界".repeat(MAX_FIELD_VALUE_BYTES));
        assert!(record.event.len() <= MAX_EVENT_BYTES);
        assert!(record.event.is_char_boundary(record.event.len()));
        assert!(record.message.len() <= MAX_MESSAGE_BYTES);
        assert!(record.fields["context"].len() <= MAX_FIELD_VALUE_BYTES);
        assert!(record.message.is_char_boundary(record.message.len()));
        assert!(record.fields["context"].is_char_boundary(record.fields["context"].len()));
    }

    #[test]
    fn buffer_never_keeps_more_than_its_capacity() {
        let mut buffer = DiagnosticBuffer::with_capacity(1);
        for _ in 0..32 {
            buffer.push(DiagnosticRecord::new(
                DiagnosticLevel::Info,
                "runtime.tick",
                "tick completed",
            ));
        }
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.dropped_records(), 31);
    }
}
