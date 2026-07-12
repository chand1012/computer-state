use crate::model::MetricSample;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};
use sysinfo::{Disks, Networks, System};

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

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
struct GpuCollector;

#[cfg(any(target_os = "windows", target_os = "linux"))]
impl GpuCollector {
    fn new() -> Self {
        Self {
            nvml: nvml_wrapper::Nvml::init().ok(),
        }
    }

    fn available(&self) -> bool {
        self.nvml
            .as_ref()
            .is_some_and(|nvml| nvml.device_count().is_ok_and(|count| count > 0))
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
            if enabled.contains("gpu.usage") {
                if let Ok(utilization) = device.utilization_rates() {
                    push(
                        samples,
                        "gpu.usage",
                        utilization.gpu as f64,
                        timestamp,
                        labels.clone(),
                    );
                }
            }
            if enabled.contains("gpu.memory.total") || enabled.contains("gpu.memory.usage") {
                if let Ok(memory) = device.memory_info() {
                    if enabled.contains("gpu.memory.total") {
                        push(
                            samples,
                            "gpu.memory.total",
                            memory.total as f64,
                            timestamp,
                            labels.clone(),
                        );
                    }
                    if enabled.contains("gpu.memory.usage") {
                        push(
                            samples,
                            "gpu.memory.usage",
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

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl GpuCollector {
    fn new() -> Self {
        Self
    }
    fn available(&self) -> bool {
        false
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

    pub async fn gpu_available(&self) -> Result<bool, String> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .gpu
                .available()
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
