//! Tracing capture layer for per-action log capture.
//!
//! Captures tracing events during action execution for later inspection.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// A captured trace event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedTrace {
    /// When the event was captured.
    pub timestamp: DateTime<Utc>,
    /// Log level: "DEBUG", "INFO", "WARN", "ERROR", "TRACE".
    pub level: String,
    /// Module path (target) where the event originated.
    pub target: String,
    /// The log message.
    pub message: String,
    /// Additional fields from the event as JSON.
    pub fields: serde_json::Value,
    /// Nested span names from outermost to innermost.
    pub span_path: Vec<String>,
}

/// Buffer for collecting captured traces.
#[derive(Debug, Default, Clone)]
pub struct CaptureBuffer {
    traces: Arc<Mutex<Vec<CapturedTrace>>>,
}

impl CaptureBuffer {
    /// Create a new empty capture buffer.
    pub fn new() -> Self {
        Self {
            traces: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Take all captured traces, clearing the buffer.
    pub fn take(&self) -> Vec<CapturedTrace> {
        let mut traces = self.traces.lock().expect("capture buffer poisoned");
        std::mem::take(&mut *traces)
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.traces
            .lock()
            .expect("capture buffer poisoned")
            .is_empty()
    }

    /// Get count of captured traces.
    pub fn len(&self) -> usize {
        self.traces.lock().expect("capture buffer poisoned").len()
    }

    /// Push a trace to the buffer.
    fn push(&self, trace: CapturedTrace) {
        self.traces
            .lock()
            .expect("capture buffer poisoned")
            .push(trace);
    }
}

/// Tracing layer that captures events to a buffer.
pub struct ActionCaptureLayer {
    buffer: CaptureBuffer,
    min_level: Level,
}

impl ActionCaptureLayer {
    /// Create a new capture layer with default level (DEBUG).
    pub fn new(buffer: CaptureBuffer) -> Self {
        Self {
            buffer,
            min_level: Level::DEBUG,
        }
    }

    /// Create with a minimum capture level.
    pub fn with_level(buffer: CaptureBuffer, level: Level) -> Self {
        Self {
            buffer,
            min_level: level,
        }
    }
}

/// Visitor that extracts fields from a tracing event.
struct FieldVisitor {
    message: Option<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl FieldVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: serde_json::Map::new(),
        }
    }
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let value_str = format!("{:?}", value);
        if field.name() == "message" {
            self.message = Some(value_str);
        } else {
            self.fields
                .insert(field.name().to_string(), serde_json::Value::String(value_str));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.fields
                .insert(field.name().to_string(), serde_json::Value::Number(n));
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
}

impl<S> Layer<S> for ActionCaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Check if event level meets minimum
        let event_level = *event.metadata().level();
        if event_level > self.min_level {
            return;
        }

        // Extract fields using visitor
        let mut visitor = FieldVisitor::new();
        event.record(&mut visitor);

        // Build span path by walking up the span stack
        let mut span_path = Vec::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                span_path.push(span.name().to_string());
            }
        }
        // Reverse to get outermost first
        span_path.reverse();

        let trace = CapturedTrace {
            timestamp: Utc::now(),
            level: level_to_string(event_level),
            target: event.metadata().target().to_string(),
            message: visitor.message.unwrap_or_default(),
            fields: serde_json::Value::Object(visitor.fields),
            span_path,
        };

        self.buffer.push(trace);
    }
}

/// Convert a tracing Level to a string.
fn level_to_string(level: Level) -> String {
    match level {
        Level::TRACE => "TRACE".to_string(),
        Level::DEBUG => "DEBUG".to_string(),
        Level::INFO => "INFO".to_string(),
        Level::WARN => "WARN".to_string(),
        Level::ERROR => "ERROR".to_string(),
    }
}

/// Format captured traces for display.
pub fn format_captured_traces(traces: &[CapturedTrace]) -> String {
    let mut output = String::new();

    for trace in traces {
        let timestamp = trace.timestamp.format("%H:%M:%S%.3f");
        let level_colored = match trace.level.as_str() {
            "ERROR" => format!("\x1b[31m{}\x1b[0m", trace.level),
            "WARN" => format!("\x1b[33m{}\x1b[0m", trace.level),
            "INFO" => format!("\x1b[32m{}\x1b[0m", trace.level),
            "DEBUG" => format!("\x1b[34m{}\x1b[0m", trace.level),
            "TRACE" => format!("\x1b[35m{}\x1b[0m", trace.level),
            _ => trace.level.clone(),
        };

        let span_prefix = if trace.span_path.is_empty() {
            String::new()
        } else {
            format!("[{}] ", trace.span_path.join("::"))
        };

        let fields_str = if trace.fields.as_object().map_or(true, |m| m.is_empty()) {
            String::new()
        } else {
            format!(" {}", trace.fields)
        };

        let _ = writeln!(
            output,
            "{} {:>5} {}{}: {}{}",
            timestamp, level_colored, span_prefix, trace.target, trace.message, fields_str
        );
    }

    output
}

/// Serialize traces to JSON for storage.
pub fn serialize_traces(traces: &[CapturedTrace]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(traces)?)
}

/// Deserialize traces from JSON.
pub fn deserialize_traces(json: &str) -> anyhow::Result<Vec<CapturedTrace>> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info_span;
    use tracing_subscriber::prelude::*;

    #[test]
    fn test_capture_buffer_operations() {
        let buffer = CaptureBuffer::new();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);

        let trace = CapturedTrace {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "test message".to_string(),
            fields: serde_json::Value::Object(serde_json::Map::new()),
            span_path: vec![],
        };

        buffer.push(trace);
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 1);

        let traces = buffer.take();
        assert_eq!(traces.len(), 1);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_capture_tracing_events() {
        let buffer = CaptureBuffer::new();
        let layer = ActionCaptureLayer::new(buffer.clone());

        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(key = "value", "test info message");
            tracing::warn!("test warning");
            tracing::error!(code = 42, "test error");
        });

        let traces = buffer.take();
        assert_eq!(traces.len(), 3);

        // Check info message
        assert_eq!(traces[0].level, "INFO");
        assert_eq!(traces[0].message, "test info message");
        assert_eq!(traces[0].fields["key"], "value");

        // Check warning
        assert_eq!(traces[1].level, "WARN");
        assert_eq!(traces[1].message, "test warning");

        // Check error with numeric field
        assert_eq!(traces[2].level, "ERROR");
        assert_eq!(traces[2].message, "test error");
        assert_eq!(traces[2].fields["code"], 42);
    }

    #[test]
    fn test_capture_with_spans() {
        let buffer = CaptureBuffer::new();
        let layer = ActionCaptureLayer::new(buffer.clone());

        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let outer = info_span!("outer_span");
            let _outer_guard = outer.enter();

            let inner = info_span!("inner_span");
            let _inner_guard = inner.enter();

            tracing::info!("nested message");
        });

        let traces = buffer.take();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].span_path, vec!["outer_span", "inner_span"]);
    }

    #[test]
    fn test_level_filtering() {
        let buffer = CaptureBuffer::new();
        let layer = ActionCaptureLayer::with_level(buffer.clone(), Level::WARN);

        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!("debug message");
            tracing::info!("info message");
            tracing::warn!("warn message");
            tracing::error!("error message");
        });

        let traces = buffer.take();
        // Only WARN and ERROR should be captured
        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].level, "WARN");
        assert_eq!(traces[1].level, "ERROR");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let traces = vec![
            CapturedTrace {
                timestamp: Utc::now(),
                level: "INFO".to_string(),
                target: "test::module".to_string(),
                message: "first message".to_string(),
                fields: serde_json::json!({"key": "value"}),
                span_path: vec!["span1".to_string()],
            },
            CapturedTrace {
                timestamp: Utc::now(),
                level: "ERROR".to_string(),
                target: "test::error".to_string(),
                message: "error message".to_string(),
                fields: serde_json::json!({"code": 500}),
                span_path: vec!["span1".to_string(), "span2".to_string()],
            },
        ];

        let json = serialize_traces(&traces).expect("serialization failed");
        let deserialized = deserialize_traces(&json).expect("deserialization failed");

        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized[0].level, "INFO");
        assert_eq!(deserialized[0].message, "first message");
        assert_eq!(deserialized[0].target, "test::module");
        assert_eq!(deserialized[0].span_path, vec!["span1"]);

        assert_eq!(deserialized[1].level, "ERROR");
        assert_eq!(deserialized[1].message, "error message");
        assert_eq!(deserialized[1].fields["code"], 500);
    }

    #[test]
    fn test_format_captured_traces() {
        let traces = vec![CapturedTrace {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "test message".to_string(),
            fields: serde_json::Value::Object(serde_json::Map::new()),
            span_path: vec![],
        }];

        let output = format_captured_traces(&traces);
        assert!(output.contains("INFO"));
        assert!(output.contains("test message"));
        assert!(output.contains("test"));
    }

    #[test]
    fn test_format_with_fields_and_spans() {
        let traces = vec![CapturedTrace {
            timestamp: Utc::now(),
            level: "WARN".to_string(),
            target: "bombadil::link".to_string(),
            message: "conflict detected".to_string(),
            fields: serde_json::json!({"file": "/home/user/.bashrc"}),
            span_path: vec!["link".to_string(), "process_dot".to_string()],
        }];

        let output = format_captured_traces(&traces);
        assert!(output.contains("WARN"));
        assert!(output.contains("link::process_dot"));
        assert!(output.contains("conflict detected"));
        assert!(output.contains(".bashrc"));
    }

    #[test]
    fn test_buffer_thread_safety() {
        use std::thread;

        let buffer = CaptureBuffer::new();
        let buffer_clone = buffer.clone();

        let handle = thread::spawn(move || {
            for i in 0..100 {
                buffer_clone.push(CapturedTrace {
                    timestamp: Utc::now(),
                    level: "INFO".to_string(),
                    target: "thread".to_string(),
                    message: format!("message {}", i),
                    fields: serde_json::Value::Object(serde_json::Map::new()),
                    span_path: vec![],
                });
            }
        });

        for i in 0..100 {
            buffer.push(CapturedTrace {
                timestamp: Utc::now(),
                level: "INFO".to_string(),
                target: "main".to_string(),
                message: format!("message {}", i),
                fields: serde_json::Value::Object(serde_json::Map::new()),
                span_path: vec![],
            });
        }

        handle.join().expect("thread panicked");

        let traces = buffer.take();
        assert_eq!(traces.len(), 200);
    }
}
