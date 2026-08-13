//! Metrics — Prometheus-compatible counters, gauges, and histograms.
//!
//! This module provides in-process metrics without external dependencies.
//! Exposed via GET /api/metrics in Prometheus text format.

#![allow(missing_docs)]

use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};

/// Thread-safe metrics registry.
#[derive(Default)]
pub struct MetricsRegistry {
    counters: RwLock<Vec<Counter>>,
    gauges: RwLock<Vec<Gauge>>,
    histograms: RwLock<Vec<Histogram>>,
    /// Counters with dynamic label values (RFC-024).
    ///
    /// Each (name, label_key, label_value) triple is a unique time series.
    /// `LabeledCounterHandle` stores the index into this vec, and increment
    /// is a single `fetch_add` on the stored atomic. O(1) per inc, O(n) only
    /// at registration.
    labeled_counters: RwLock<Vec<LabeledCounter>>,
}

struct LabeledCounter {
    name: String,
    help: String,
    label_key: &'static str,
    label_value: String,
    value: AtomicU64,
}

impl MetricsRegistry {
    /// Create a new metrics registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new counter and return a handle.
    pub fn counter(
        &self,
        name: &'static str,
        help: &'static str,
        labels: &[(&'static str, &'static str)],
    ) -> CounterHandle {
        let mut counters = self.counters.write();
        let id = counters.len();
        counters.push(Counter {
            name: name.into(),
            help: help.into(),
            labels: labels.into(),
            value: AtomicU64::new(0),
        });
        CounterHandle { id }
    }

    /// Register a new gauge and return a handle.
    pub fn gauge(&self, name: &'static str, help: &'static str, initial: f64) -> GaugeHandle {
        let mut gauges = self.gauges.write();
        let id = gauges.len();
        gauges.push(Gauge {
            name: name.into(),
            help: help.into(),
            value: Mutex::new(initial),
        });
        GaugeHandle { id }
    }

    /// Register a new histogram and return a handle.
    pub fn histogram(
        &self,
        name: &'static str,
        help: &'static str,
        buckets: Vec<f64>,
    ) -> HistogramHandle {
        let mut histograms = self.histograms.write();
        let id = histograms.len();
        let counts: Vec<usize> = vec![0; buckets.len() + 1];
        histograms.push(Histogram {
            name: name.into(),
            help: help.into(),
            buckets: buckets.clone(),
            counts: RwLock::new(counts),
            sum: Mutex::new(0.0),
            count: Mutex::new(0u64),
        });
        HistogramHandle { id, buckets }
    }
    /// Register a labeled counter (RFC-024) — one time series per
    /// (name, label_key, label_value) triple. O(1) increment, registration
    /// is O(n) over existing labeled counters with the same name (linear
    /// scan; metric name sets are small at boot).
    pub fn labeled_counter(
        &self,
        name: &'static str,
        help: &'static str,
        label_key: &'static str,
        label_value: &str,
    ) -> LabeledCounterHandle {
        let mut labeled = self.labeled_counters.write();
        // Reuse the existing series if registered with the same triple.
        for (i, lc) in labeled.iter().enumerate() {
            if lc.name == name && lc.label_key == label_key && lc.label_value == label_value {
                return LabeledCounterHandle { id: i };
            }
        }
        let id = labeled.len();
        labeled.push(LabeledCounter {
            name: name.into(),
            help: help.into(),
            label_key,
            label_value: label_value.into(),
            value: AtomicU64::new(0),
        });
        LabeledCounterHandle { id }
    }

    /// Export all metrics in Prometheus text format.
    pub fn export(&self) -> String {
        let mut out = String::new();

        // Counters
        {
            let counters = self.counters.read();
            for c in counters.iter() {
                out.push_str(&format!("# HELP {} {}\n", c.name, c.help));
                out.push_str(&format!("# TYPE {} counter\n", c.name));
                let value = c.value.load(Ordering::Relaxed);
                let labels_str = if c.labels.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{{{}}}",
                        c.labels
                            .iter()
                            .map(|(k, v)| format!("{k}=\"{v}\""))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                out.push_str(&format!("{}{} {}\n", c.name, labels_str, value));
            }
        }

        // Gauges
        {
            let gauges = self.gauges.read();
            for g in gauges.iter() {
                out.push_str(&format!("# HELP {} {}\n", g.name, g.help));
                out.push_str(&format!("# TYPE {} gauge\n", g.name));
                let value = *g.value.lock();
                out.push_str(&format!("{} {}\n", g.name, value));
            }
        }

        // Histograms
        {
            let histograms = self.histograms.read();
            for h in histograms.iter() {
                out.push_str(&format!("# HELP {} {}\n", h.name, h.help));
                out.push_str(&format!("# TYPE {} histogram\n", h.name));
                let sum = *h.sum.lock();
                let count = *h.count.lock();
                let bucket_values = h.buckets.clone();
                let counts = h.counts.read();
                let mut cumulative = 0usize;
                for (i, _) in bucket_values.iter().enumerate() {
                    cumulative += counts[i];
                    let boundary = bucket_values[i];
                    out.push_str(&format!(
                        "{}{{le=\"{}\"}} {}\n",
                        h.name, boundary, cumulative
                    ));
                }
                // +Inf bucket
                out.push_str(&format!("{}{{le=\"+Inf\"}} {}\n", h.name, cumulative));
                out.push_str(&format!("{}_sum {} \n", h.name, sum));
                out.push_str(&format!("{}_count {} \n", h.name, count));
            }
        }

        // Labeled counters (RFC-024). One series per registered
        // (name, label_key, label_value) triple. HELP/TYPE lines are emitted
        // per-series (Prometheus accepts repeated HELP/TYPE per series).
        {
            let labeled = self.labeled_counters.read();
            let mut seen_help: std::collections::HashSet<String> = std::collections::HashSet::new();
            for lc in labeled.iter() {
                if seen_help.insert(lc.name.clone()) {
                    out.push_str(&format!("# HELP {} {}\n", lc.name, lc.help));
                    out.push_str(&format!("# TYPE {} counter\n", lc.name));
                }
                out.push_str(&format!(
                    "{}{{{}=\"{}\"}} {}\n",
                    lc.name,
                    lc.label_key,
                    lc.label_value,
                    lc.value.load(Ordering::Relaxed)
                ));
            }
        }

        out
    }
}

/// Global metrics registry.
static REGISTRY: std::sync::OnceLock<MetricsRegistry> = std::sync::OnceLock::new();

/// Get the global metrics registry.
pub fn registry() -> &'static MetricsRegistry {
    REGISTRY.get_or_init(MetricsRegistry::new)
}

#[derive(Clone)]
pub struct CounterHandle {
    id: usize,
}

impl CounterHandle {
    /// Increment the counter by 1.
    pub fn inc(&self) {
        let r = registry();
        let counters = r.counters.read();
        if let Some(c) = counters.get(self.id) {
            c.value.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Increment the counter by `n`.
    pub fn inc_by(&self, n: u64) {
        if n == 0 {
            return;
        }
        let r = registry();
        let counters = r.counters.read();
        if let Some(c) = counters.get(self.id) {
            c.value.fetch_add(n, Ordering::Relaxed);
        }
    }
}

/// Handle to a labeled counter (RFC-024). Each handle refers to one
/// (name, label_key, label_value) time series.
#[derive(Clone)]
pub struct LabeledCounterHandle {
    id: usize,
}

impl LabeledCounterHandle {
    /// Increment the counter by 1.
    pub fn inc(&self) {
        let r = registry();
        let labeled = r.labeled_counters.read();
        if let Some(c) = labeled.get(self.id) {
            c.value.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Increment the counter by `n`.
    pub fn inc_by(&self, n: u64) {
        if n == 0 {
            return;
        }
        let r = registry();
        let labeled = r.labeled_counters.read();
        if let Some(c) = labeled.get(self.id) {
            c.value.fetch_add(n, Ordering::Relaxed);
        }
    }
}

#[derive(Clone)]
pub struct GaugeHandle {
    id: usize,
}

impl GaugeHandle {
    /// Set the gauge to a specific value.
    pub fn set(&self, v: f64) {
        let r = registry();
        let gauges = r.gauges.read();
        if let Some(g) = gauges.get(self.id) {
            *g.value.lock() = v;
        }
    }

    /// Increment the gauge by 1.
    pub fn inc(&self) {
        let r = registry();
        let gauges = r.gauges.read();
        if let Some(g) = gauges.get(self.id) {
            let mut val = g.value.lock();
            *val += 1.0;
        }
    }

    /// Decrement the gauge by 1.
    pub fn dec(&self) {
        let r = registry();
        let gauges = r.gauges.read();
        if let Some(g) = gauges.get(self.id) {
            let mut val = g.value.lock();
            *val -= 1.0;
        }
    }
}

#[derive(Clone)]
pub struct HistogramHandle {
    id: usize,
    buckets: Vec<f64>,
}

impl HistogramHandle {
    /// Observe a value, adding it to the histogram.
    pub fn observe(&self, value: f64) {
        let r = registry();
        let histograms = r.histograms.read();
        if let Some(h) = histograms.get(self.id) {
            {
                let mut sum = h.sum.lock();
                *sum += value;
            }
            {
                let mut count = h.count.lock();
                *count += 1;
            }
            {
                let mut counts = h.counts.write();
                for (i, boundary) in self.buckets.iter().enumerate() {
                    if value <= *boundary {
                        counts[i] += 1;
                    }
                }
                // +Inf bucket
                counts[self.buckets.len()] += 1;
            }
        }
    }
}

struct Counter {
    name: String,
    help: String,
    labels: Vec<(&'static str, &'static str)>,
    value: AtomicU64,
}

struct Gauge {
    name: String,
    help: String,
    value: Mutex<f64>,
}

struct Histogram {
    name: String,
    help: String,
    buckets: Vec<f64>,
    counts: RwLock<Vec<usize>>,
    sum: Mutex<f64>,
    count: Mutex<u64>,
}

/// Metrics handles initialized at startup.
#[derive(Clone)]
pub struct MetricsHandles {
    pub agents_forked: CounterHandle,
    pub agents_completed: CounterHandle,
    pub agents_failed: CounterHandle,
    /// RFC-027 retry metrics.
    pub retry_attempted: CounterHandle,
    pub retry_improved: CounterHandle,
    pub retry_unchanged: CounterHandle,
    pub retry_degraded: CounterHandle,
    pub orch_duration: HistogramHandle,
    pub messages: CounterHandle,
    /// LLM circuit breaker state: 0=closed, 1=open, 2=half_open.
    pub llm_circuit_breaker_state: GaugeHandle,
    /// Tool execution metrics.
    pub tool_calls: CounterHandle,
    pub tool_errors: CounterHandle,
    pub tool_duration: HistogramHandle,
    /// LLM call metrics.
    pub llm_calls: CounterHandle,
    pub llm_errors: CounterHandle,
    pub audit_lagged_events: CounterHandle,

    // ── RFC-024: web↔daemon reliability metrics ──
    // Labeled counters use one handle per (name, label_key, label_value)
    // triple. Wire them up at the increment sites in oxios-gateway and the
    // web routes — see RFC-024 §11.
    /// Outgoing messages by outcome (delivered | dropped | resynced | timed_out).
    pub gateway_messages_delivered: LabeledCounterHandle,
    pub gateway_messages_dropped: LabeledCounterHandle,
    pub gateway_messages_resynced: LabeledCounterHandle,
    pub gateway_messages_timed_out: LabeledCounterHandle,

    /// Replay requests by outcome (replay | resync).
    pub gateway_replay_replay: LabeledCounterHandle,
    pub gateway_replay_resync: LabeledCounterHandle,

    /// `send_and_wait` duration histogram.
    pub gateway_response_duration: HistogramHandle,

    /// SSE client connections by action (open | close). The original
    /// RFC-024 §11 metric (`sse_reconnects_total{reason}`) required
    /// client-side observability (proxy/NAT/UA-specific reasons) the
    /// server cannot see. We expose server-side lifecycle instead.
    pub sse_connections_open: LabeledCounterHandle,
    pub sse_connections_close: LabeledCounterHandle,

    /// WS client connections by action (open | close | keepalive_timeout).
    pub ws_connections_open: LabeledCounterHandle,
    pub ws_connections_close: LabeledCounterHandle,
    pub ws_connections_keepalive_timeout: LabeledCounterHandle,

    /// Atomic web-dist swaps (RFC-024 SP3).
    pub web_dist_swaps: CounterHandle,
    /// Web surface scoped restarts (RFC-030).
    pub supervisor_restarts: CounterHandle,

    /// 0 = warming up, 1 = ready (RFC-024 SP4). Updated from
    /// `ReadinessGate` when a subsystem changes state.
    pub readiness_state: GaugeHandle,

    /// Brain daemon reachable (1/0, RFC-047). Set from boot after the
    /// initial `BrainConnection::connect`.
    pub oxibrain_available: GaugeHandle,

    /// Brain recall operations (RFC-047). Incremented on each successful
    /// `BrainConnection::recall`.
    pub oxibrain_recall_total: CounterHandle,
}

impl MetricsHandles {
    /// Increment agents_forked counter.
    pub fn inc_agents_forked(&self) {
        self.agents_forked.inc();
    }

    /// Increment agents_completed counter.
    pub fn inc_agents_completed(&self) {
        self.agents_completed.inc();
    }

    /// Increment agents_failed counter.
    pub fn inc_agents_failed(&self) {
        self.agents_failed.inc();
    }

    /// Increment messages counter.
    pub fn inc_messages(&self) {
        self.messages.inc();
    }
    /// Increment the supervisor scoped-restart counter (RFC-030).
    pub fn inc_supervisor_restart(&self) {
        self.supervisor_restarts.inc();
    }

    /// Observe orchestration duration.
    pub fn observe_orch_duration(&self, value: f64) {
        self.orch_duration.observe(value);
    }
}
/// Global lazy metric handles.
static METRICS: std::sync::OnceLock<MetricsHandles> = std::sync::OnceLock::new();

/// Get or create the metrics handles.
pub fn get_metrics() -> &'static MetricsHandles {
    METRICS.get_or_init(|| {
        let r = registry();
        MetricsHandles {
            agents_forked: r.counter("oxios_agents_forked_total", "Total agents forked", &[]),
            agents_completed: r.counter(
                "oxios_agents_completed_total",
                "Total agents completed",
                &[],
            ),
            agents_failed: r.counter("oxios_agents_failed_total", "Total agents failed", &[]),
            retry_attempted: r.counter(
                "oxios_retry_attempted_total",
                "Review retries attempted",
                &[],
            ),
            retry_improved: r.counter(
                "oxios_retry_improved_total",
                "Retries that improved score",
                &[],
            ),
            retry_unchanged: r.counter(
                "oxios_retry_unchanged_total",
                "Retries with same score",
                &[],
            ),
            retry_degraded: r.counter(
                "oxios_retry_degraded_total",
                "Retries that degraded score",
                &[],
            ),
            orch_duration: r.histogram(
                "oxios_orchestration_duration_seconds",
                "Orchestration duration",
                vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0],
            ),
            messages: r.counter("oxios_messages_processed_total", "Messages processed", &[]),
            llm_circuit_breaker_state: r.gauge(
                "oxios_llm_circuit_breaker_state",
                "LLM circuit breaker state: 0=closed, 1=open, 2=half_open",
                0.0,
            ),
            tool_calls: r.counter("oxios_tool_calls_total", "Tool calls", &[]),
            tool_errors: r.counter("oxios_tool_errors_total", "Tool errors", &[]),
            tool_duration: r.histogram(
                "oxios_tool_duration_seconds",
                "Tool call duration",
                vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
            ),
            llm_calls: r.counter("oxios_llm_calls_total", "LLM API calls", &[]),
            llm_errors: r.counter("oxios_llm_errors_total", "LLM API errors", &[]),
            audit_lagged_events: r.counter(
                "oxios_audit_lagged_events_total",
                "Audit events dropped due to broadcast subscriber lag",
                &[],
            ),

            // ── RFC-024: web↔daemon reliability metrics ──
            // One handle per (name, label_value) variant. The registry's
            // labeled_counter dedup ensures each variant registers exactly
            // once even if both `get_metrics()` and `register_builtin_metrics`
            // are called at startup.
            gateway_messages_delivered: r.labeled_counter(
                "oxios_gateway_messages_total",
                "Outgoing messages (result=delivered|dropped|resynced|timed_out)",
                "result",
                "delivered",
            ),
            gateway_messages_dropped: r.labeled_counter(
                "oxios_gateway_messages_total",
                "Outgoing messages (result=delivered|dropped|resynced|timed_out)",
                "result",
                "dropped",
            ),
            gateway_messages_resynced: r.labeled_counter(
                "oxios_gateway_messages_total",
                "Outgoing messages (result=delivered|dropped|resynced|timed_out)",
                "result",
                "resynced",
            ),
            gateway_messages_timed_out: r.labeled_counter(
                "oxios_gateway_messages_total",
                "Outgoing messages (result=delivered|dropped|resynced|timed_out)",
                "result",
                "timed_out",
            ),
            gateway_replay_replay: r.labeled_counter(
                "oxios_gateway_replay_requests_total",
                "Replay requests (outcome=replay|resync)",
                "outcome",
                "replay",
            ),
            gateway_replay_resync: r.labeled_counter(
                "oxios_gateway_replay_requests_total",
                "Replay requests (outcome=replay|resync)",
                "outcome",
                "resync",
            ),
            gateway_response_duration: r.histogram(
                "oxios_gateway_response_duration_seconds",
                "send_and_wait duration in seconds",
                vec![0.05, 0.25, 1.0, 5.0, 30.0, 60.0, 120.0],
            ),
            sse_connections_open: r.labeled_counter(
                "oxios_sse_connections_total",
                "SSE client connections (action=open|close)",
                "action",
                "open",
            ),
            sse_connections_close: r.labeled_counter(
                "oxios_sse_connections_total",
                "SSE client connections (action=open|close)",
                "action",
                "close",
            ),
            ws_connections_open: r.labeled_counter(
                "oxios_ws_connections_total",
                "WS client connections (action=open|close|keepalive_timeout)",
                "action",
                "open",
            ),
            ws_connections_close: r.labeled_counter(
                "oxios_ws_connections_total",
                "WS client connections (action=open|close|keepalive_timeout)",
                "action",
                "close",
            ),
            ws_connections_keepalive_timeout: r.labeled_counter(
                "oxios_ws_connections_total",
                "WS client connections (action=open|close|keepalive_timeout)",
                "action",
                "keepalive_timeout",
            ),
            web_dist_swaps: r.counter(
                "oxios_web_dist_swaps_total",
                "Atomic web-dist swaps (RFC-024 SP3)",
                &[],
            ),
            supervisor_restarts: r.counter(
                "oxios_supervisor_restarts_total",
                "Web surface scoped restarts (RFC-030)",
                &[],
            ),
            readiness_state: r.gauge(
                "oxios_readiness_state",
                "0 = warming up, 1 = ready (RFC-024 SP4)",
                0.0,
            ),
            oxibrain_available: r.gauge("oxibrain_available", "Brain daemon reachable (1/0)", 0.0),
            oxibrain_recall_total: r.counter(
                "oxibrain_recall_total",
                "Brain recall operations",
                &[],
            ),
        }
    })
}

/// Register all built-in metrics. Call once at startup.
pub fn register_builtin_metrics() {
    let r = registry();

    // Agent metrics
    r.counter("oxios_agents_forked_total", "Total agents forked", &[]);
    r.gauge("oxios_agents_running", "Currently running agents", 0.0);
    r.counter(
        "oxios_agents_completed_total",
        "Total agents completed",
        &[],
    );
    r.counter("oxios_agents_failed_total", "Total agents failed", &[]);

    // Message metrics
    r.counter(
        "oxios_messages_processed_total",
        "User messages processed",
        &[],
    );
    r.histogram(
        "oxios_orchestration_duration_seconds",
        "Full orchestration duration",
        vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0],
    );

    r.histogram(
        "oxios_phase_duration_seconds",
        "Phase duration",
        vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0],
    );

    // LLM metrics
    r.counter("oxios_llm_calls_total", "LLM API calls", &[]);
    r.histogram(
        "oxios_llm_duration_seconds",
        "LLM call duration",
        vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0],
    );
    r.counter("oxios_llm_errors_total", "LLM API errors", &[]);
    // Audit pipeline metric (state-area F4): events silently dropped when
    // the audit trail subscriber falls behind the broadcast bus.
    r.counter(
        "oxios_audit_lagged_events_total",
        "Audit events dropped due to broadcast subscriber lag",
        &[],
    );

    // Tool metrics
    r.counter("oxios_tool_calls_total", "Tool calls", &[]);
    r.histogram(
        "oxios_tool_duration_seconds",
        "Tool call duration",
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
    );
    r.counter("oxios_tool_errors_total", "Tool errors", &[]);

    // Brain daemon metrics (RFC-047). Both are typed handles in
    // get_metrics() — set from boot (availability) and incremented on recall.

    // Container metrics
    r.counter("oxios_exec_total", "Exec calls", &[]);
    r.histogram(
        "oxios_exec_duration_seconds",
        "Exec duration",
        vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0],
    );

    // Session metrics
    r.gauge("oxios_active_sessions", "Active sessions", 0.0);

    // Initialize get_metrics() handles
    let _ = get_metrics();
}
