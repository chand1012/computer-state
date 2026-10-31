use crate::model::MetricSample;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};
use sysinfo::{Disks, Networks, System};

const GPU_USAGE: &str = "gpu.usage";
const GPU_MEMORY_TOTAL: &str = "gpu.memory.total";
const GPU_MEMORY_USAGE: &str = "gpu.memory.usage";
const GPU_METRICS: [&str; 3] = [GPU_USAGE, GPU_MEMORY_TOTAL, GPU_MEMORY_USAGE];

pub struct SystemCollector {
    inner: Arc<Mutex<CollectorInner>>,
}

struct CollectorInner {
    system: System,
    networks: Networks,
    disks: Disks,
    gpu: GpuCollector,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
struct GpuCollector {
    nvml: Option<nvml_wrapper::Nvml>,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
struct GpuCollector;

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    all(target_os = "macos", target_arch = "aarch64")
)))]
struct GpuCollector;

#[cfg(any(target_os = "windows", target_os = "linux"))]
impl GpuCollector {
    fn new() -> Self {
        Self {
            nvml: nvml_wrapper::Nvml::init().ok(),
        }
    }

    fn capabilities(&self) -> HashSet<String> {
        let available = self
            .nvml
            .as_ref()
            .is_some_and(|nvml| nvml.device_count().is_ok_and(|count| count > 0));
        if available {
            GPU_METRICS.iter().map(|metric| (*metric).into()).collect()
        } else {
            HashSet::new()
        }
    }

    fn collect(&self, enabled: &HashSet<String>, timestamp: i64, samples: &mut Vec<MetricSample>) {
        let Some(nvml) = &self.nvml else { return };
        let Ok(count) = nvml.device_count() else {
            return;
        };
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            let mut labels = BTreeMap::new();
            labels.insert("gpu".into(), index.to_string());
            if let Ok(name) = device.name() {
                labels.insert("name".into(), name);
            }
            if enabled.contains(GPU_USAGE) {
                if let Ok(utilization) = device.utilization_rates() {
                    push(
                        samples,
                        GPU_USAGE,
                        utilization.gpu as f64,
                        timestamp,
                        labels.clone(),
                    );
                }
            }
            if enabled.contains(GPU_MEMORY_TOTAL) || enabled.contains(GPU_MEMORY_USAGE) {
                if let Ok(memory) = device.memory_info() {
                    if enabled.contains(GPU_MEMORY_TOTAL) {
                        push(
                            samples,
                            GPU_MEMORY_TOTAL,
                            memory.total as f64,
                            timestamp,
                            labels.clone(),
                        );
                    }
                    if enabled.contains(GPU_MEMORY_USAGE) {
                        push(
                            samples,
                            GPU_MEMORY_USAGE,
                            memory.used as f64,
                            timestamp,
                            labels.clone(),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
impl GpuCollector {
    fn new() -> Self {
        Self
    }

    fn capabilities(&self) -> HashSet<String> {
        let mut capabilities = HashSet::new();
        let Some(device) = apple_silicon_device() else {
            return capabilities;
        };

        if device.recommended_max_working_set_size() > 0 {
            capabilities.insert(GPU_MEMORY_TOTAL.into());
        }
        if let Some(statistics) = performance_statistics() {
            if utilization(&statistics).is_some() {
                capabilities.insert(GPU_USAGE.into());
            }
            if used_memory(&statistics).is_some() {
                capabilities.insert(GPU_MEMORY_USAGE.into());
            }
        }
        capabilities
    }

    fn collect(&self, enabled: &HashSet<String>, timestamp: i64, samples: &mut Vec<MetricSample>) {
        if !gpu_metrics_enabled(enabled) {
            return;
        }
        let Some(device) = apple_silicon_device() else {
            return;
        };

        let mut labels = BTreeMap::new();
        labels.insert("gpu".into(), "0".into());
        labels.insert("name".into(), device.name().to_string());
        labels.insert("memory_type".into(), "unified".into());

        if enabled.contains(GPU_MEMORY_TOTAL) {
            let budget = device.recommended_max_working_set_size();
            if budget > 0 {
                let mut total_labels = labels.clone();
                total_labels.insert("capacity_basis".into(), "recommended_working_set".into());
                push(
                    samples,
                    GPU_MEMORY_TOTAL,
                    budget as f64,
                    timestamp,
                    total_labels,
                );
            }
        }

        if !performance_statistics_enabled(enabled) {
            return;
        }
        let Some(statistics) = performance_statistics() else {
            return;
        };
        if enabled.contains(GPU_USAGE) {
            if let Some(value) = utilization(&statistics) {
                push(samples, GPU_USAGE, value, timestamp, labels.clone());
            }
        }
        if enabled.contains(GPU_MEMORY_USAGE) {
            if let Some(value) = used_memory(&statistics) {
                push(samples, GPU_MEMORY_USAGE, value as f64, timestamp, labels);
            }
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn apple_silicon_device() -> Option<metal::Device> {
    metal::Device::system_default().filter(|device| device.has_unified_memory())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn performance_statistics() -> Option<BTreeMap<String, iokit::cf::CFValue>> {
    let candidates = iokit::io_service::matching_services("IOAccelerator")
        .ok()?
        .into_iter()
        .filter_map(|service| {
            let entry = service.registry_entry().ok()?;
            match entry.property("PerformanceStatistics").ok()?? {
                iokit::cf::CFValue::Dictionary(statistics) => Some(statistics),
                _ => None,
            }
        });
    select_performance_statistics(candidates)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn select_performance_statistics(
    candidates: impl IntoIterator<Item = BTreeMap<String, iokit::cf::CFValue>>,
) -> Option<BTreeMap<String, iokit::cf::CFValue>> {
    candidates
        .into_iter()
        .find(|statistics| utilization(statistics).is_some() || used_memory(statistics).is_some())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn utilization(statistics: &BTreeMap<String, iokit::cf::CFValue>) -> Option<f64> {
    match statistics.get("Device Utilization %")? {
        iokit::cf::CFValue::Integer(value) => Some((*value as f64).clamp(0.0, 100.0)),
        _ => None,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn used_memory(statistics: &BTreeMap<String, iokit::cf::CFValue>) -> Option<u64> {
    match statistics.get("In use system memory")? {
        iokit::cf::CFValue::Integer(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), test))]
fn gpu_metrics_enabled(enabled: &HashSet<String>) -> bool {
    GPU_METRICS.iter().any(|metric| enabled.contains(*metric))
}

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), test))]
fn performance_statistics_enabled(enabled: &HashSet<String>) -> bool {
    enabled.contains(GPU_USAGE) || enabled.contains(GPU_MEMORY_USAGE)
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    all(target_os = "macos", target_arch = "aarch64")
)))]
impl GpuCollector {
    fn new() -> Self {
        Self
    }
    fn capabilities(&self) -> HashSet<String> {
        HashSet::new()
    }
    fn collect(
        &self,
        _enabled: &HashSet<String>,
        _timestamp: i64,
        _samples: &mut Vec<MetricSample>,
    ) {
    }
}

impl SystemCollector {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CollectorInner {
                system: System::new_all(),
                networks: Networks::new_with_refreshed_list(),
                disks: Disks::new_with_refreshed_list(),
                gpu: GpuCollector::new(),
            })),
        }
    }

    pub async fn gpu_capabilities(&self) -> Result<HashSet<String>, String> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .gpu
                .capabilities()
        })
        .await
        .map_err(|error| format!("GPU capability task failed: {error}"))
    }

    pub async fn collect(&self, enabled: HashSet<String>) -> Result<Vec<MetricSample>, String> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut collector = inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            collect_blocking(&enabled, &mut collector)
        })
        .await
        .map_err(|error| format!("collector task failed: {error}"))
    }
}

fn collect_blocking(
    enabled: &HashSet<String>,
    collector: &mut CollectorInner,
) -> Vec<MetricSample> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    collector.system.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    collector.system.refresh_cpu_usage();
    let mut samples = Vec::new();

    if enabled.contains("cpu.total.usage") {
        push(
            &mut samples,
            "cpu.total.usage",
            collector.system.global_cpu_usage() as f64,
            timestamp,
            BTreeMap::new(),
        );
    }
    if enabled.contains("cpu.core.usage") {
        for (index, cpu) in collector.system.cpus().iter().enumerate() {
            let mut labels = BTreeMap::new();
            labels.insert("core".into(), index.to_string());
            push(
                &mut samples,
                "cpu.core.usage",
                cpu.cpu_usage() as f64,
                timestamp,
                labels,
            );
        }
    }
    if enabled.contains("memory.total") {
        push(
            &mut samples,
            "memory.total",
            collector.system.total_memory() as f64,
            timestamp,
            BTreeMap::new(),
        );
    }
    if enabled.contains("memory.usage") {
        push(
            &mut samples,
            "memory.usage",
            collector.system.used_memory() as f64,
            timestamp,
            BTreeMap::new(),
        );
    }
    if enabled.contains("process.count") {
        push(
            &mut samples,
            "process.count",
            collector.system.processes().len() as f64,
            timestamp,
            BTreeMap::new(),
        );
    }
    if enabled.contains("uptime") {
        push(
            &mut samples,
            "uptime",
            System::uptime() as f64,
            timestamp,
            BTreeMap::new(),
        );
    }

    if enabled.contains("network.bytes_received") || enabled.contains("network.bytes_sent") {
        collector.networks.refresh(true);
        for (name, network) in &collector.networks {
            let mut labels = BTreeMap::new();
            labels.insert("interface".into(), name.to_string());
            if enabled.contains("network.bytes_received") {
                push(
                    &mut samples,
                    "network.bytes_received",
                    network.total_received() as f64,
                    timestamp,
                    labels.clone(),
                );
            }
            if enabled.contains("network.bytes_sent") {
                push(
                    &mut samples,
                    "network.bytes_sent",
                    network.total_transmitted() as f64,
                    timestamp,
                    labels,
                );
            }
        }
    }

    if enabled.contains("storage.total") || enabled.contains("storage.usage") {
        collector.disks.refresh(true);
        for disk in &collector.disks {
            let mut labels = BTreeMap::new();
            labels.insert(
                "volume".into(),
                disk.mount_point().to_string_lossy().into_owned(),
            );
            let total = disk.total_space();
            if enabled.contains("storage.total") {
                push(
                    &mut samples,
                    "storage.total",
                    total as f64,
                    timestamp,
                    labels.clone(),
                );
            }
            if enabled.contains("storage.usage") {
                push(
                    &mut samples,
                    "storage.usage",
                    total.saturating_sub(disk.available_space()) as f64,
                    timestamp,
                    labels,
                );
            }
        }
    }
    collector.gpu.collect(enabled, timestamp, &mut samples);
    samples
}

fn push(
    samples: &mut Vec<MetricSample>,
    metric: &str,
    value: f64,
    timestamp: i64,
    labels: BTreeMap<String, String>,
) {
    samples.push(MetricSample {
        metric: metric.into(),
        timestamp,
        value,
        labels,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_metric_checks_skip_unneeded_work() {
        let cpu_only = HashSet::from(["cpu.total.usage".to_string()]);
        assert!(!gpu_metrics_enabled(&cpu_only));
        assert!(!performance_statistics_enabled(&cpu_only));

        let total_only = HashSet::from([GPU_MEMORY_TOTAL.to_string()]);
        assert!(gpu_metrics_enabled(&total_only));
        assert!(!performance_statistics_enabled(&total_only));

        let usage = HashSet::from([GPU_USAGE.to_string()]);
        assert!(gpu_metrics_enabled(&usage));
        assert!(performance_statistics_enabled(&usage));
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    mod macos {
        use super::*;
        use iokit::cf::CFValue;

        fn statistics(
            entries: impl IntoIterator<Item = (&'static str, CFValue)>,
        ) -> BTreeMap<String, CFValue> {
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect()
        }

        #[test]
        fn parses_complete_performance_statistics() {
            let values = statistics([
                ("Device Utilization %", CFValue::Integer(42)),
                ("In use system memory", CFValue::Integer(1_234)),
            ]);

            assert_eq!(utilization(&values), Some(42.0));
            assert_eq!(used_memory(&values), Some(1_234));
        }

        #[test]
        fn handles_missing_and_wrong_performance_statistics() {
            let missing = BTreeMap::new();
            assert_eq!(utilization(&missing), None);
            assert_eq!(used_memory(&missing), None);

            let wrong = statistics([
                ("Device Utilization %", CFValue::String("busy".into())),
                ("In use system memory", CFValue::Boolean(true)),
            ]);
            assert_eq!(utilization(&wrong), None);
            assert_eq!(used_memory(&wrong), None);
        }

        #[test]
        fn clamps_utilization_and_rejects_negative_memory() {
            let high = statistics([("Device Utilization %", CFValue::Integer(150))]);
            let low = statistics([("Device Utilization %", CFValue::Integer(-10))]);
            let memory = statistics([("In use system memory", CFValue::Integer(-1))]);

            assert_eq!(utilization(&high), Some(100.0));
            assert_eq!(utilization(&low), Some(0.0));
            assert_eq!(used_memory(&memory), None);
        }

        #[test]
        fn selects_first_valid_accelerator_statistics() {
            let invalid = statistics([("Unrelated", CFValue::Integer(1))]);
            let first_valid = statistics([("Device Utilization %", CFValue::Integer(20))]);
            let second_valid = statistics([("Device Utilization %", CFValue::Integer(80))]);

            let selected =
                select_performance_statistics([invalid, first_valid.clone(), second_valid])
                    .unwrap();
            assert_eq!(selected, first_valid);
        }
    }
}
