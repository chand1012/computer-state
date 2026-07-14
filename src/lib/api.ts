import { invoke } from "@tauri-apps/api/core";

export type MetricSample = {
  metric: string;
  timestamp: number;
  value: number;
  labels: Record<string, string>;
};

export type MetricDescriptor = {
  id: string;
  label: string;
  unit: "percent" | "bytes" | "seconds" | "processes";
  available: boolean;
  reason?: string;
};

export type AppSettings = {
  version: number;
  collection: { interval_seconds: number; retention_days: number };
  http: { port: number; allowed_interfaces: string[] };
  metrics: Record<string, boolean>;
};

export type ServiceStatus = {
  database_ready: boolean;
  http_port: number;
  allowed_interfaces: string[];
  latest_collection?: string;
};

export type HttpLogEntry = {
  timestamp: string;
  level: "INFO" | "ERROR";
  message: string;
};

export const api = {
  settings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) => invoke<AppSettings>("update_settings", { settings }),
  startupEnabled: () => invoke<boolean>("get_startup_enabled"),
  setStartupEnabled: (enabled: boolean) => invoke<boolean>("set_startup_enabled", { enabled }),
  catalog: () => invoke<MetricDescriptor[]>("get_metric_catalog"),
  latest: () => invoke<MetricSample[]>("get_latest_metrics"),
  status: () => invoke<ServiceStatus>("get_service_status"),
  httpLogs: () => invoke<HttpLogEntry[]>("get_http_logs"),
  history: (metrics: string[], from: Date, to: Date, aggregation?: string, intervalSeconds?: number) =>
    invoke<MetricSample[]>("query_metric_history", {
      query: { metrics, from: from.toISOString(), to: to.toISOString(), aggregation, intervalSeconds },
    }),
};

export function formatMetric(value: number, unit: MetricDescriptor["unit"]) {
  if (unit === "percent") return `${value.toFixed(1)}%`;
  if (unit === "bytes") {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let amount = value; let index = 0;
    while (Math.abs(amount) >= 1024 && index < units.length - 1) { amount /= 1024; index += 1; }
    return `${amount.toFixed(index ? 1 : 0)} ${units[index]}`;
  }
  if (unit === "seconds") {
    const days = Math.floor(value / 86400);
    const hours = Math.floor((value % 86400) / 3600);
    return days ? `${days}d ${hours}h` : `${hours}h`;
  }
  return Math.round(value).toLocaleString();
}
