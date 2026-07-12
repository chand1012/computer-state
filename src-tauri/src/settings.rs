use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::model::METRICS;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSettings {
    pub interval_seconds: u64,
    pub retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpSettings {
    pub port: u16,
    pub allowed_interfaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub version: u32,
    pub collection: CollectionSettings,
    pub http: HttpSettings,
    pub metrics: BTreeMap<String, bool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            collection: CollectionSettings {
                interval_seconds: 60,
                retention_days: 7,
            },
            http: HttpSettings {
                port: 8888,
                allowed_interfaces: vec!["loopback".into(), "tailscale".into()],
            },
            metrics: METRICS
                .iter()
                .map(|(id, _, _)| ((*id).to_string(), true))
                .collect(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported settings version".into());
        }
        if !(10..=86_400).contains(&self.collection.interval_seconds) {
            return Err("collection interval must be between 10 seconds and 24 hours".into());
        }
        if !(1..=365).contains(&self.collection.retention_days) {
            return Err("retention must be between 1 and 365 days".into());
        }
        if self.http.allowed_interfaces.is_empty() {
            return Err("at least one allowed interface is required".into());
        }
        for interface in &self.http.allowed_interfaces {
            if interface != "loopback"
                && interface != "tailscale"
                && interface.parse::<std::net::IpAddr>().is_err()
            {
                return Err(format!("invalid allowed interface: {interface}"));
            }
        }
        Ok(())
    }

    fn migrated(mut self) -> Self {
        let defaults = Self::default();
        for (metric, enabled) in defaults.metrics {
            self.metrics.entry(metric).or_insert(enabled);
        }
        self
    }
}

pub struct SettingsService {
    path: PathBuf,
    value: RwLock<AppSettings>,
}

impl SettingsService {
    pub async fn load(path: PathBuf) -> Result<Self, String> {
        let value = match tokio::fs::read_to_string(&path).await {
            Ok(contents) => match serde_json::from_str::<AppSettings>(&contents) {
                Ok(value) => value.migrated(),
                Err(error) => {
                    let backup = path.with_file_name(format!(
                        "settings.invalid-{}.json",
                        chrono::Utc::now().timestamp()
                    ));
                    tokio::fs::rename(&path, &backup)
                        .await
                        .map_err(|rename_error| format!("invalid settings file ({error}) and failed to preserve it: {rename_error}"))?;
                    tracing::error!(%error, backup = %backup.display(), "invalid settings preserved; safe defaults restored");
                    AppSettings::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
            Err(error) => return Err(format!("failed to read settings: {error}")),
        };
        value.validate()?;
        let service = Self {
            path,
            value: RwLock::new(value),
        };
        if !service.path.exists() {
            service.persist_current().await?;
        }
        Ok(service)
    }

    pub async fn get(&self) -> AppSettings {
        self.value.read().await.clone()
    }

    pub async fn replace(&self, value: AppSettings) -> Result<(), String> {
        value.validate()?;
        self.persist(&value).await?;
        *self.value.write().await = value;
        Ok(())
    }

    async fn persist_current(&self) -> Result<(), String> {
        self.persist(&self.get().await).await
    }

    async fn persist(&self, value: &AppSettings) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| format!("failed to create settings: {error}"))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| format!("failed to write settings: {error}"))?;
        file.flush()
            .await
            .map_err(|error| format!("failed to flush settings: {error}"))?;
        file.sync_all()
            .await
            .map_err(|error| format!("failed to sync settings: {error}"))?;
        drop(file);
        tokio::fs::rename(&temporary, &self.path)
            .await
            .map_err(|error| format!("failed to commit settings: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_product_contract() {
        let settings = AppSettings::default();
        assert_eq!(settings.collection.interval_seconds, 60);
        assert_eq!(settings.collection.retention_days, 7);
        assert_eq!(settings.http.port, 8888);
        assert_eq!(settings.http.allowed_interfaces, ["loopback", "tailscale"]);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn rejects_unsafe_or_abusive_values() {
        let mut settings = AppSettings::default();
        settings.collection.interval_seconds = 1;
        assert!(settings.validate().is_err());
        settings = AppSettings::default();
        settings.http.allowed_interfaces.clear();
        assert!(settings.validate().is_err());
    }

    #[tokio::test]
    async fn preserves_malformed_settings_and_recovers_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        tokio::fs::write(&path, b"{not valid json").await.unwrap();
        let service = SettingsService::load(path).await.unwrap();
        assert_eq!(service.get().await.http.port, 8888);
        let mut entries = tokio::fs::read_dir(directory.path()).await.unwrap();
        let mut found_backup = false;
        while let Some(entry) = entries.next_entry().await.unwrap() {
            found_backup |= entry
                .file_name()
                .to_string_lossy()
                .starts_with("settings.invalid-");
        }
        assert!(found_backup);
    }
}
