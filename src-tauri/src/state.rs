use crate::{
    collector::SystemCollector, database::MetricRepository, model::MetricSample,
    settings::SettingsService,
};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, RwLock};

pub struct AppState {
    pub settings: Arc<SettingsService>,
    pub repository: Arc<MetricRepository>,
    pub latest: RwLock<Vec<MetricSample>>,
    pub scheduler_changed: Notify,
    collector: SystemCollector,
    server: Mutex<Option<ServerRuntime>>,
}

struct ServerRuntime {
    shutdown: rocket::Shutdown,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl AppState {
    pub async fn metric_descriptors(&self) -> Result<Vec<crate::model::MetricDescriptor>, String> {
        Ok(crate::model::descriptors(
            self.collector.gpu_available().await?,
        ))
    }
    pub async fn collect(&self, persist: bool) -> Result<Vec<MetricSample>, String> {
        let settings = self.settings.get().await;
        let enabled: HashSet<String> = settings
            .metrics
            .into_iter()
            .filter_map(|(metric, enabled)| enabled.then_some(metric))
            .collect();
        let samples = self.collector.collect(enabled).await?;
        if persist {
            self.repository.insert(&samples).await?;
            *self.latest.write().await = samples.clone();
        }
        Ok(samples)
    }

    pub async fn cleanup(&self) -> Result<u64, String> {
        let days = self.settings.get().await.collection.retention_days as i64;
        let cutoff = chrono::Utc::now().timestamp_millis() - days * 86_400_000;
        self.repository.cleanup(cutoff).await
    }

    pub fn new(
        settings: Arc<SettingsService>,
        repository: Arc<MetricRepository>,
        latest: Vec<MetricSample>,
    ) -> Self {
        Self {
            settings,
            repository,
            latest: RwLock::new(latest),
            scheduler_changed: Notify::new(),
            collector: SystemCollector::new(),
            server: Mutex::new(None),
        }
    }

    pub async fn restart_http(self: &Arc<Self>) -> Result<(), String> {
        let mut server = self.server.lock().await;
        if let Some(previous) = server.take() {
            previous.shutdown.notify();
            let _ = tokio::time::timeout(Duration::from_secs(2), previous.task).await;
        }
        let port = self.settings.get().await.http.port;
        let state = self.clone();
        let rocket = crate::http::build(state, port)
            .ignite()
            .await
            .map_err(|error| format!("failed to initialize HTTP server: {error}"))?;
        let shutdown = rocket.shutdown();
        let task = tauri::async_runtime::spawn(async move {
            if let Err(error) = rocket.launch().await {
                tracing::error!(%error, port, "HTTP server stopped");
            }
        });
        *server = Some(ServerRuntime { shutdown, task });
        Ok(())
    }

    pub async fn shutdown_http(&self) {
        if let Some(server) = self.server.lock().await.take() {
            server.shutdown.notify();
            let _ = tokio::time::timeout(Duration::from_secs(2), server.task).await;
        }
    }
}

pub fn spawn_scheduler(state: Arc<AppState>, app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = state.collect(true).await;
        let mut last_cleanup = std::time::Instant::now();
        loop {
            let interval = state.settings.get().await.collection.interval_seconds;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {
                    if state.collect(true).await.is_ok() {
                        use tauri::Emitter;
                        let _ = app.emit("metrics://sample-created", chrono::Utc::now().timestamp_millis());
                    }
                    if last_cleanup.elapsed() >= Duration::from_secs(86_400) {
                        let _ = state.cleanup().await;
                        last_cleanup = std::time::Instant::now();
                    }
                }
                _ = state.scheduler_changed.notified() => {}
            }
        }
    });
}
