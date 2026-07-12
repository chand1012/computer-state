mod collector;
mod database;
mod http;
mod model;
mod settings;
mod state;

use chrono::{DateTime, Utc};
use model::MetricSample;
use serde::{Deserialize, Serialize};
use settings::AppSettings;
use state::AppState;
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[tauri::command]
async fn get_settings(state: tauri::State<'_, Arc<AppState>>) -> Result<AppSettings, String> {
    Ok(state.settings.get().await)
}

#[tauri::command]
async fn update_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let previous = state.settings.get().await;
    settings.validate()?;
    if settings.http.port != previous.http.port {
        std::net::TcpListener::bind(("0.0.0.0", settings.http.port))
            .map_err(|error| format!("cannot listen on port {}: {error}", settings.http.port))?;
    }
    state.settings.replace(settings).await?;
    state.scheduler_changed.notify_one();
    state.restart_http().await?;
    let _ = state.cleanup().await;
    let updated = state.settings.get().await;
    app.emit("settings://updated", &updated)
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

#[tauri::command]
async fn get_metric_catalog(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<model::MetricDescriptor>, String> {
    state.metric_descriptors().await
}

#[tauri::command]
async fn get_latest_metrics(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<MetricSample>, String> {
    let settings = state.settings.get().await;
    Ok(state
        .latest
        .read()
        .await
        .iter()
        .filter(|sample| {
            settings
                .metrics
                .get(&sample.metric)
                .copied()
                .unwrap_or(false)
        })
        .cloned()
        .collect())
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    metrics: Vec<String>,
    from: String,
    to: String,
    aggregation: Option<String>,
    interval_seconds: Option<i64>,
}

#[tauri::command]
async fn query_metric_history(
    state: tauri::State<'_, Arc<AppState>>,
    mut query: HistoryQuery,
) -> Result<Vec<MetricSample>, String> {
    query.metrics.sort();
    query.metrics.dedup();
    if query.metrics.is_empty()
        || query
            .metrics
            .iter()
            .any(|metric| !model::is_known_metric(metric))
    {
        return Err("one or more valid metrics are required".into());
    }
    if query.metrics.len() > 16 {
        return Err("a query may request at most 16 metrics".into());
    }
    let from = DateTime::parse_from_rfc3339(&query.from)
        .map_err(|_| "invalid from timestamp")?
        .with_timezone(&Utc)
        .timestamp_millis();
    let to = DateTime::parse_from_rfc3339(&query.to)
        .map_err(|_| "invalid to timestamp")?
        .with_timezone(&Utc)
        .timestamp_millis();
    if from >= to {
        return Err("from must precede to".into());
    }
    let retention = state.settings.get().await.collection.retention_days as i64 * 86_400_000;
    let from = from.max(Utc::now().timestamp_millis() - retention);
    if let Some(operation) = query.aggregation.as_deref() {
        state
            .repository
            .aggregate(
                &query.metrics,
                from,
                to,
                operation,
                query.interval_seconds.map(|value| value * 1000),
            )
            .await
    } else {
        state
            .repository
            .raw(
                &query.metrics,
                from,
                to,
                50_000 / query.metrics.len() as i64,
            )
            .await
    }
}

#[derive(Serialize)]
struct ServiceStatus {
    database_ready: bool,
    http_port: u16,
    allowed_interfaces: Vec<String>,
    latest_collection: Option<String>,
}

#[tauri::command]
async fn get_service_status(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ServiceStatus, String> {
    let settings = state.settings.get().await;
    let latest_collection = state
        .latest
        .read()
        .await
        .iter()
        .map(|sample| sample.timestamp)
        .max()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|value| value.to_rfc3339());
    Ok(ServiceStatus {
        database_ready: true,
        http_port: settings.http.port,
        allowed_interfaces: settings.http.allowed_interfaces,
        latest_collection,
    })
}

fn open_route(app: &tauri::AppHandle, route: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = app.emit("navigation://requested", route);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_route(app, "/metrics");
        }))
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;
            let handle = app.handle().clone();
            let app_state = tauri::async_runtime::block_on(async {
                let settings = Arc::new(
                    settings::SettingsService::load(app_data.join("settings.json"))
                        .await
                        .map_err(std::io::Error::other)?,
                );
                let repository = Arc::new(
                    database::MetricRepository::open(&app_data.join("computer-state.sqlite3"))
                        .await
                        .map_err(std::io::Error::other)?,
                );
                let enabled = settings
                    .get()
                    .await
                    .metrics
                    .into_iter()
                    .filter_map(|(metric, enabled)| enabled.then_some(metric))
                    .collect::<Vec<_>>();
                let latest = repository
                    .latest(&enabled)
                    .await
                    .map_err(std::io::Error::other)?;
                let state = Arc::new(AppState::new(settings, repository, latest));
                state.cleanup().await.map_err(std::io::Error::other)?;
                state.restart_http().await.map_err(std::io::Error::other)?;
                Ok::<_, std::io::Error>(state)
            })?;
            state::spawn_scheduler(app_state.clone(), handle.clone());
            app.manage(app_state);

            let metrics =
                tauri::menu::MenuItem::with_id(app, "metrics", "Metrics", true, None::<&str>)?;
            let settings =
                tauri::menu::MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&metrics, &settings, &quit])?;
            let mut tray = tauri::tray::TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(true);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "metrics" => open_route(app, "/metrics"),
                "settings" => open_route(app, "/settings"),
                "quit" => {
                    let app = app.clone();
                    let state = app.state::<Arc<AppState>>().inner().clone();
                    tauri::async_runtime::spawn(async move {
                        state.shutdown_http().await;
                        app.exit(0);
                    });
                }
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            get_metric_catalog,
            get_latest_metrics,
            query_metric_history,
            get_service_status
        ])
        .build(tauri::generate_context!())
        .expect("error while building Computer State");

    application.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            let state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::block_on(state.shutdown_http());
        }
    });
}
