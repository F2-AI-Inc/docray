use crate::worker::WorkerOutcome;
use docray_model::OutputFormat;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _, UpDownCounter};
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::Resource;
use serde_json::{json, Map};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const EVENT_SCHEMA_VERSION: u64 = 1;

#[derive(Clone)]
pub struct Telemetry {
    inner: Arc<Inner>,
}

struct Inner {
    json_events: bool,
    service_name: String,
    provider: Option<SdkMeterProvider>,
    requests: Counter<u64>,
    errors: Counter<u64>,
    timeouts: Counter<u64>,
    request_duration: Histogram<f64>,
    queue_duration: Histogram<f64>,
    extraction_duration: Histogram<f64>,
    input_size: Histogram<u64>,
    output_size: Histogram<u64>,
    pages: Counter<u64>,
    records: Counter<u64>,
    warnings: Counter<u64>,
    textless_pages: Counter<u64>,
    active: UpDownCounter<i64>,
    active_count: AtomicU64,
}

impl Telemetry {
    pub fn from_env() -> Result<Self, String> {
        let json_events = parse_json_events(
            &std::env::var("DOCRAY_TELEMETRY_LOGS").unwrap_or_else(|_| "off".into()),
        )?;
        let otlp_enabled = parse_metrics_exporters(
            &std::env::var("OTEL_METRICS_EXPORTER").unwrap_or_else(|_| "none".into()),
        )?;
        let service_name =
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "docray-server".into());

        let provider = if otlp_enabled {
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .build()
                .map_err(|error| error.to_string())?;
            let resource = Resource::builder()
                .with_service_name(service_name.clone())
                .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
                .build();
            Some(
                SdkMeterProvider::builder()
                    .with_resource(resource)
                    .with_periodic_exporter(exporter)
                    .build(),
            )
        } else {
            None
        };

        let meter = match &provider {
            Some(provider) => provider.meter("docray-server"),
            None => opentelemetry::global::meter("docray-server"),
        };
        let requests = meter
            .u64_counter("docray.extraction.requests")
            .with_description("Extraction requests completed")
            .with_unit("{request}")
            .build();
        let errors = meter
            .u64_counter("docray.extraction.errors")
            .with_description("Extraction requests completed with an error")
            .with_unit("{error}")
            .build();
        let timeouts = meter
            .u64_counter("docray.extraction.timeouts")
            .with_description("Extraction requests terminated by the service timeout")
            .with_unit("{timeout}")
            .build();
        let request_duration = meter
            .f64_histogram("docray.extraction.request.duration")
            .with_description("End-to-end extraction request duration")
            .with_unit("s")
            .build();
        let queue_duration = meter
            .f64_histogram("docray.extraction.queue.duration")
            .with_description("Time a synchronous extraction waited for a worker slot")
            .with_unit("s")
            .build();
        let extraction_duration = meter
            .f64_histogram("docray.extraction.worker.duration")
            .with_description("Time spent running the extraction worker")
            .with_unit("s")
            .build();
        let input_size = meter
            .u64_histogram("docray.extraction.input.size")
            .with_description("Uploaded document size")
            .with_unit("By")
            .build();
        let output_size = meter
            .u64_histogram("docray.extraction.output.size")
            .with_description("Successful extraction response size")
            .with_unit("By")
            .build();
        let pages = meter
            .u64_counter("docray.extraction.pages")
            .with_description("Pages emitted by successful extractions")
            .with_unit("{page}")
            .build();
        let records = meter
            .u64_counter("docray.extraction.records")
            .with_description("Element records emitted by successful extractions")
            .with_unit("{record}")
            .build();
        let warnings = meter
            .u64_counter("docray.extraction.warnings")
            .with_description("Extraction warnings emitted")
            .with_unit("{warning}")
            .build();
        let textless_pages = meter
            .u64_counter("docray.extraction.textless_pages")
            .with_description("LEAN pages emitted without readable text content")
            .with_unit("{page}")
            .build();
        let active = meter
            .i64_up_down_counter("docray.extraction.active")
            .with_description("Extractions currently running")
            .with_unit("{request}")
            .build();

        Ok(Self {
            inner: Arc::new(Inner {
                json_events,
                service_name,
                provider,
                requests,
                errors,
                timeouts,
                request_duration,
                queue_duration,
                extraction_duration,
                input_size,
                output_size,
                pages,
                records,
                warnings,
                textless_pages,
                active,
                active_count: AtomicU64::new(0),
            }),
        })
    }

    pub fn begin_extraction(&self, route: &'static str) -> ActiveExtraction {
        let current = self.inner.active_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner
            .active
            .add(1, &[KeyValue::new("docray.route", route)]);
        ActiveExtraction {
            telemetry: self.clone(),
            route,
            current,
        }
    }

    pub fn record(&self, metric: &ExtractionMetric) {
        let attributes = metric.attributes();
        self.inner.requests.add(1, &attributes);
        self.inner
            .request_duration
            .record(metric.request_duration.as_secs_f64(), &attributes);
        if metric.outcome != "success" {
            self.inner.errors.add(1, &attributes);
        }
        if metric.outcome == "timeout" {
            self.inner.timeouts.add(1, &attributes);
        }
        if let Some(duration) = metric.queue_duration {
            self.inner
                .queue_duration
                .record(duration.as_secs_f64(), &attributes);
        }
        if let Some(duration) = metric.extraction_duration {
            self.inner
                .extraction_duration
                .record(duration.as_secs_f64(), &attributes);
        }
        if let Some(bytes) = metric.input_bytes {
            self.inner.input_size.record(bytes, &attributes);
        }
        if let Some(stats) = &metric.output {
            self.inner
                .output_size
                .record(stats.output_bytes, &attributes);
            if let Some(count) = stats.page_count {
                self.inner.pages.add(count, &attributes);
            }
            for (kind, count) in stats.record_counts() {
                let mut record_attributes = attributes.clone();
                record_attributes.push(KeyValue::new("docray.record.type", kind));
                self.inner.records.add(count, &record_attributes);
            }
            if let Some(count) = stats.warning_count {
                if count > 0 {
                    self.inner.warnings.add(count, &attributes);
                }
            }
            if let Some(count) = stats.textless_page_count {
                if count > 0 {
                    self.inner.textless_pages.add(count, &attributes);
                }
            }
        }

        if self.inner.json_events {
            println!("{}", metric.to_json_line(&self.inner.service_name));
        }
    }

    pub fn shutdown(&self) -> Result<(), String> {
        match &self.inner.provider {
            Some(provider) => provider.shutdown().map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }
}

fn parse_json_events(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "false" | "0" | "no" | "" => Ok(false),
        "json" | "true" | "1" | "yes" => Ok(true),
        other => Err(format!(
            "DOCRAY_TELEMETRY_LOGS must be 'off' or 'json', got {other:?}"
        )),
    }
}

fn parse_metrics_exporters(value: &str) -> Result<bool, String> {
    let exporters = value
        .split(',')
        .map(str::trim)
        .filter(|exporter| !exporter.is_empty())
        .collect::<Vec<_>>();
    if exporters.is_empty() || (exporters.len() == 1 && exporters[0].eq_ignore_ascii_case("none")) {
        return Ok(false);
    }
    if exporters
        .iter()
        .all(|exporter| exporter.eq_ignore_ascii_case("otlp"))
    {
        return Ok(true);
    }
    Err(format!(
        "OTEL_METRICS_EXPORTER supports 'none' or 'otlp', got {value:?}"
    ))
}

pub struct ActiveExtraction {
    telemetry: Telemetry,
    route: &'static str,
    current: u64,
}

impl ActiveExtraction {
    pub fn current(&self) -> u64 {
        self.current
    }
}

impl Drop for ActiveExtraction {
    fn drop(&mut self) {
        self.telemetry
            .inner
            .active_count
            .fetch_sub(1, Ordering::Relaxed);
        self.telemetry
            .inner
            .active
            .add(-1, &[KeyValue::new("docray.route", self.route)]);
    }
}

pub struct ExtractionMetric {
    pub route: &'static str,
    pub format: &'static str,
    pub granularity: &'static str,
    pub classify: bool,
    pub outcome: &'static str,
    pub error_code: Option<String>,
    pub status_code: Option<u16>,
    pub request_duration: Duration,
    pub queue_duration: Option<Duration>,
    pub extraction_duration: Option<Duration>,
    pub input_bytes: Option<u64>,
    pub output: Option<OutputStats>,
    pub in_flight: Option<u64>,
}

impl ExtractionMetric {
    pub fn new(route: &'static str) -> Self {
        Self {
            route,
            format: "unknown",
            granularity: "unknown",
            classify: false,
            outcome: "error",
            error_code: Some("request_incomplete".into()),
            status_code: None,
            request_duration: Duration::ZERO,
            queue_duration: None,
            extraction_duration: None,
            input_bytes: None,
            output: None,
            in_flight: None,
        }
    }

    pub fn fail(&mut self, code: impl Into<String>) {
        self.outcome = "error";
        self.error_code = Some(code.into());
    }

    pub fn observe_outcome(&mut self, outcome: &WorkerOutcome, format: OutputFormat) {
        match outcome {
            WorkerOutcome::Success(bytes) => {
                self.outcome = "success";
                self.error_code = None;
                self.output = Some(OutputStats::from_output(format, bytes));
            }
            WorkerOutcome::Failed { code, .. } => self.fail(code.clone()),
            WorkerOutcome::Timeout => {
                self.outcome = "timeout";
                self.error_code = Some("timeout".into());
            }
            WorkerOutcome::Crashed => {
                self.outcome = "crash";
                self.error_code = Some("crash".into());
            }
            WorkerOutcome::OutputTooLarge => {
                self.outcome = "error";
                self.error_code = Some("output_too_large".into());
            }
        }
    }

    fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = vec![
            KeyValue::new("docray.route", self.route),
            KeyValue::new("docray.format", self.format),
            KeyValue::new("docray.granularity", self.granularity),
            KeyValue::new("docray.classify", self.classify),
            KeyValue::new("docray.outcome", self.outcome),
        ];
        if let Some(code) = &self.error_code {
            attributes.push(KeyValue::new("error.type", code.clone()));
        }
        if let Some(status_code) = self.status_code {
            attributes.push(KeyValue::new(
                "http.response.status_code",
                i64::from(status_code),
            ));
        }
        if let Some(stats) = &self.output {
            if let Some(version) = &stats.schema_version {
                attributes.push(KeyValue::new("docray.schema.version", version.clone()));
            }
        }
        attributes
    }

    fn to_json_line(&self, service_name: &str) -> String {
        serde_json::to_string(&self.to_json_value(service_name))
            .unwrap_or_else(|_| "{\"event\":\"docray.telemetry.serialization_error\"}".into())
    }

    fn to_json_value(&self, service_name: &str) -> serde_json::Value {
        let mut fields = Map::new();
        fields.insert("event".into(), json!("docray.extraction.completed"));
        fields.insert("event_schema_version".into(), json!(EVENT_SCHEMA_VERSION));
        fields.insert("timestamp_unix_ms".into(), json!(unix_time_ms()));
        fields.insert("service_name".into(), json!(service_name));
        fields.insert("service_version".into(), json!(env!("CARGO_PKG_VERSION")));
        fields.insert("route".into(), json!(self.route));
        fields.insert("format".into(), json!(self.format));
        fields.insert("granularity".into(), json!(self.granularity));
        fields.insert("classify".into(), json!(self.classify));
        fields.insert("outcome".into(), json!(self.outcome));
        fields.insert(
            "request_duration_ms".into(),
            json!(duration_ms(self.request_duration)),
        );
        insert_optional(&mut fields, "error_code", self.error_code.as_ref());
        insert_optional(&mut fields, "status_code", self.status_code.as_ref());
        insert_optional(
            &mut fields,
            "queue_duration_ms",
            self.queue_duration.map(duration_ms).as_ref(),
        );
        insert_optional(
            &mut fields,
            "extraction_duration_ms",
            self.extraction_duration.map(duration_ms).as_ref(),
        );
        insert_optional(&mut fields, "input_bytes", self.input_bytes.as_ref());
        insert_optional(&mut fields, "in_flight", self.in_flight.as_ref());
        if let Some(stats) = &self.output {
            fields.insert("output_bytes".into(), json!(stats.output_bytes));
            insert_optional(&mut fields, "page_count", stats.page_count.as_ref());
            insert_optional(
                &mut fields,
                "text_record_count",
                stats.text_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "image_record_count",
                stats.image_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "path_record_count",
                stats.path_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "chart_record_count",
                stats.chart_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "table_record_count",
                stats.table_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "table_cell_record_count",
                stats.table_cell_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "annotation_record_count",
                stats.annotation_record_count.as_ref(),
            );
            insert_optional(
                &mut fields,
                "word_record_count",
                stats.word_record_count.as_ref(),
            );
            insert_optional(&mut fields, "warning_count", stats.warning_count.as_ref());
            insert_optional(
                &mut fields,
                "textless_page_count",
                stats.textless_page_count.as_ref(),
            );
            insert_optional(&mut fields, "schema_version", stats.schema_version.as_ref());
        }
        fields.into()
    }
}

fn insert_optional<T: serde::Serialize>(
    fields: &mut Map<String, serde_json::Value>,
    key: &str,
    value: Option<&T>,
) {
    if let Some(value) = value {
        fields.insert(key.into(), json!(value));
    }
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[derive(Default)]
pub struct OutputStats {
    output_bytes: u64,
    page_count: Option<u64>,
    text_record_count: Option<u64>,
    image_record_count: Option<u64>,
    path_record_count: Option<u64>,
    chart_record_count: Option<u64>,
    table_record_count: Option<u64>,
    table_cell_record_count: Option<u64>,
    annotation_record_count: Option<u64>,
    word_record_count: Option<u64>,
    warning_count: Option<u64>,
    textless_page_count: Option<u64>,
    schema_version: Option<String>,
}

impl OutputStats {
    fn from_output(format: OutputFormat, bytes: &[u8]) -> Self {
        let mut stats = Self {
            output_bytes: bytes.len() as u64,
            ..Self::default()
        };
        match format {
            OutputFormat::Lean => stats.read_lean(bytes),
            OutputFormat::Json | OutputFormat::Markdown => {}
        }
        stats
    }

    fn read_lean(&mut self, bytes: &[u8]) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        let mut page_count = 0;
        let mut text_count = 0;
        let mut image_count = 0;
        let mut path_count = 0;
        let mut chart_count = 0;
        let mut table_count = 0;
        let mut table_cell_count = 0;
        let mut annotation_count = 0;
        let mut word_count = 0;
        let mut warning_count = 0;
        let mut current_page_has_text = false;
        let mut in_page = false;
        let mut textless_pages = 0;

        for line in text.lines() {
            if let Some(header) = line.strip_prefix("#docray ") {
                self.schema_version = header
                    .split_ascii_whitespace()
                    .find_map(|part| part.strip_prefix('v'))
                    .map(str::to_string);
            } else if line.starts_with("#warning ") {
                warning_count += 1;
            } else if line.starts_with("#page ") {
                if in_page && !current_page_has_text {
                    textless_pages += 1;
                }
                page_count += 1;
                in_page = true;
                current_page_has_text = false;
            } else if line.starts_with("T ") {
                text_count += 1;
                current_page_has_text |= lean_record_has_payload(line, 8);
            } else if line.starts_with("w ") {
                word_count += 1;
                current_page_has_text |= lean_record_has_payload(line, 5);
            } else if line.starts_with("r ") {
                current_page_has_text |= lean_record_has_payload(line, 4);
            } else if line.starts_with("I ") {
                image_count += 1;
            } else if line.starts_with("P ") {
                path_count += 1;
            } else if line.starts_with("CH ") {
                chart_count += 1;
            } else if line.starts_with("TB ") {
                table_count += 1;
            } else if line.starts_with("c ") {
                table_cell_count += 1;
                current_page_has_text |= lean_record_has_payload(line, 12);
            } else if line.starts_with("A ") {
                annotation_count += 1;
            }
        }
        if in_page && !current_page_has_text {
            textless_pages += 1;
        }
        if !in_page {
            self.warning_count = Some(warning_count);
            return;
        }
        self.page_count = Some(page_count);
        self.text_record_count = Some(text_count);
        self.image_record_count = Some(image_count);
        self.path_record_count = Some(path_count);
        self.chart_record_count = Some(chart_count);
        self.table_record_count = Some(table_count);
        self.table_cell_record_count = Some(table_cell_count);
        self.annotation_record_count = Some(annotation_count);
        self.word_record_count = Some(word_count);
        self.warning_count = Some(warning_count);
        self.textless_page_count = Some(textless_pages);
    }

    fn record_counts(&self) -> impl Iterator<Item = (&'static str, u64)> {
        [
            ("text", self.text_record_count),
            ("image", self.image_record_count),
            ("path", self.path_record_count),
            ("chart", self.chart_record_count),
            ("table", self.table_record_count),
            ("table_cell", self.table_cell_record_count),
            ("annotation", self.annotation_record_count),
            ("word", self.word_record_count),
        ]
        .into_iter()
        .filter_map(|(kind, count)| count.filter(|count| *count > 0).map(|count| (kind, count)))
    }
}

fn lean_record_has_payload(line: &str, fixed_fields: usize) -> bool {
    line.split_ascii_whitespace().count() > fixed_fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lean_stats_are_bounded_counts_only() {
        let lean = b"#docray element v1.9 pages=2 warnings=1\n#warning recovered page\n#page 1 612x792\nT 1 2 3 4 Helvetica 12 - hello\nI 0 0 10 10\nP 0 0 1 1\n#page 2 612x792\nI 0 0 10 10\n";
        let stats = OutputStats::from_output(OutputFormat::Lean, lean);
        assert_eq!(stats.page_count, Some(2));
        assert_eq!(stats.text_record_count, Some(1));
        assert_eq!(stats.image_record_count, Some(2));
        assert_eq!(stats.path_record_count, Some(1));
        assert_eq!(stats.warning_count, Some(1));
        assert_eq!(stats.textless_page_count, Some(1));
        assert_eq!(stats.schema_version.as_deref(), Some("1.9"));
    }

    #[test]
    fn non_lean_output_is_not_reparsed_for_metrics() {
        let bytes = include_bytes!("../../../testdata/golden/simple.element.json");
        let stats = OutputStats::from_output(OutputFormat::Json, bytes);
        assert_eq!(stats.output_bytes, bytes.len() as u64);
        assert_eq!(stats.page_count, None);
        assert_eq!(stats.text_record_count, None);
        assert_eq!(stats.schema_version, None);
    }

    #[test]
    fn structured_event_contains_no_document_content() {
        let mut metric = ExtractionMetric::new("sync");
        metric.format = "lean";
        metric.granularity = "element";
        metric.outcome = "success";
        metric.error_code = None;
        metric.input_bytes = Some(123);
        metric.request_duration = Duration::from_millis(50);
        metric.output = Some(OutputStats::from_output(
            OutputFormat::Lean,
            b"#docray element v1.9 pages=1\n#page 1 10x10\nT 0 0 1 1 F 1 - secret text\n",
        ));
        let encoded = metric.to_json_line("docray-test");
        assert!(encoded.contains("\"text_record_count\":1"));
        assert!(encoded.contains("\"service_name\":\"docray-test\""));
        assert!(!encoded.contains("secret text"));
    }

    #[test]
    fn telemetry_modes_validate_configuration() {
        assert!(!parse_json_events("off").unwrap());
        assert!(parse_json_events("JSON").unwrap());
        assert!(parse_json_events("xml").is_err());
        assert!(!parse_metrics_exporters("none").unwrap());
        assert!(parse_metrics_exporters("otlp").unwrap());
        assert!(parse_metrics_exporters("OTLP").unwrap());
        assert!(parse_metrics_exporters("prometheus").is_err());
    }
}
