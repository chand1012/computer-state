use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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

pub fn descriptors(available_gpu_metrics: &HashSet<String>) -> Vec<MetricDescriptor> {
    METRICS
        .iter()
        .map(|(id, label, unit)| {
            let gpu = id.starts_with("gpu.");
            let available = !gpu || available_gpu_metrics.contains(*id);
            MetricDescriptor {
                id,
                label,
                unit,
                available,
                reason: (gpu && !available)
                    .then_some("GPU telemetry is unavailable for this hardware or driver"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_report_gpu_capabilities_per_metric() {
        let capabilities = HashSet::from(["gpu.usage".to_string()]);
        let descriptors = descriptors(&capabilities);

        let usage = descriptors
            .iter()
            .find(|descriptor| descriptor.id == "gpu.usage")
            .unwrap();
        let total = descriptors
            .iter()
            .find(|descriptor| descriptor.id == "gpu.memory.total")
            .unwrap();
        let cpu = descriptors
            .iter()
            .find(|descriptor| descriptor.id == "cpu.total.usage")
            .unwrap();

        assert!(usage.available);
        assert!(usage.reason.is_none());
        assert!(!total.available);
        assert!(total.reason.is_some());
        assert!(cpu.available);
    }

    #[test]
    fn descriptors_support_empty_and_complete_gpu_capabilities() {
        let empty = descriptors(&HashSet::new());
        assert!(empty
            .iter()
            .filter(|descriptor| descriptor.id.starts_with("gpu."))
            .all(|descriptor| !descriptor.available));

        let complete = descriptors(&HashSet::from([
            "gpu.usage".to_string(),
            "gpu.memory.total".to_string(),
            "gpu.memory.usage".to_string(),
        ]));
        assert!(complete
            .iter()
            .filter(|descriptor| descriptor.id.starts_with("gpu."))
            .all(|descriptor| descriptor.available));
    }
}
