use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub metric: String,
    pub timestamp: i64,
    pub value: f64,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub available: bool,
    pub reason: Option<&'static str>,
}

pub const METRICS: &[(&str, &str, &str)] = &[
    ("cpu.core.usage", "CPU utilization per core", "percent"),
    ("cpu.total.usage", "Total CPU utilization", "percent"),
    ("gpu.usage", "GPU utilization", "percent"),
    ("gpu.memory.total", "Total GPU memory", "bytes"),
    ("gpu.memory.usage", "GPU memory utilization", "bytes"),
    ("memory.total", "Total system memory", "bytes"),
    ("memory.usage", "System memory utilization", "bytes"),
    ("network.bytes_received", "Network bytes received", "bytes"),
    ("network.bytes_sent", "Network bytes sent", "bytes"),
    ("process.count", "Running processes", "processes"),
    ("storage.total", "Total storage capacity", "bytes"),
    ("storage.usage", "Storage utilization", "bytes"),
    ("uptime", "System uptime", "seconds"),
];

pub fn is_known_metric(metric: &str) -> bool {
    METRICS.iter().any(|(id, _, _)| *id == metric)
}

pub fn descriptors(gpu_available: bool) -> Vec<MetricDescriptor> {
    METRICS
        .iter()
        .map(|(id, label, unit)| {
            let gpu = id.starts_with("gpu.");
            MetricDescriptor {
                id,
                label,
                unit,
                available: !gpu || gpu_available,
                reason: (gpu && !gpu_available)
                    .then_some("GPU telemetry is unavailable for this hardware or driver"),
            }
        })
        .collect()
}
