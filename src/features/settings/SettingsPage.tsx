import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Database, FileText, Globe2, Power, RefreshCw, Save, Server } from "lucide-react";
import { api, type AppSettings, type HttpLogEntry, type MetricDescriptor, type ServiceStatus } from "../../lib/api";
import { Button } from "../../components/ui/Button";
import { Switch } from "../../components/ui/Switch";

export function SettingsPage() {
  const [settings, setSettings] = useState<AppSettings>();
  const [catalog, setCatalog] = useState<MetricDescriptor[]>([]);
  const [status, setStatus] = useState<ServiceStatus>();
  const [httpLogs, setHttpLogs] = useState<HttpLogEntry[]>([]);
  const [loadingLogs, setLoadingLogs] = useState(false);
  const [startupEnabled, setStartupEnabled] = useState(false);
  const [changingStartup, setChangingStartup] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "ok" | "error"; text: string }>();

  useEffect(() => {
    Promise.all([api.settings(), api.catalog(), api.status(), api.startupEnabled(), api.httpLogs()])
      .then(([settings, catalog, status, startupEnabled, logs]) => { setSettings(settings); setCatalog(catalog); setStatus(status); setStartupEnabled(startupEnabled); setHttpLogs(logs); })
      .catch((error) => setMessage({ type: "error", text: String(error) }));

    let disposed = false;
    let stop: undefined | (() => void);
    listen<boolean>("startup://updated", ({ payload }) => setStartupEnabled(payload)).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    const logRefresh = window.setInterval(() => {
      api.httpLogs().then((logs) => { if (!disposed) setHttpLogs(logs); }).catch(() => undefined);
    }, 2_000);
    return () => { disposed = true; stop?.(); window.clearInterval(logRefresh); };
  }, []);

  async function refreshLogs() {
    setLoadingLogs(true);
    try { setHttpLogs(await api.httpLogs()); }
    catch (error) { setMessage({ type: "error", text: String(error) }); }
    finally { setLoadingLogs(false); }
  }

  async function changeStartup(enabled: boolean) {
    setChangingStartup(true); setMessage(undefined);
    try {
      const actual = await api.setStartupEnabled(enabled);
      setStartupEnabled(actual);
      setMessage({ type: "ok", text: actual ? "Computer State will launch at startup" : "Launch at startup disabled" });
    } catch (error) {
      setMessage({ type: "error", text: String(error) });
    } finally {
      setChangingStartup(false);
    }
  }

  async function save() {
    if (!settings) return;
    setSaving(true); setMessage(undefined);
    try {
      const next = await api.updateSettings(settings); setSettings(next); setStatus(await api.status()); setMessage({ type: "ok", text: "Settings saved" });
    } catch (error) { setMessage({ type: "error", text: String(error) }); } finally { setSaving(false); }
  }

  if (!settings) return <div className="center-state"><div className="spinner" />Loading settings…</div>;
  const setCollection = (key: "interval_seconds" | "retention_days", value: number) => setSettings({ ...settings, collection: { ...settings.collection, [key]: value } });

  return (
    <section>
      <div className="page-heading"><div><p className="eyebrow">Configuration</p><h1>Settings</h1><p>Control local collection and API access.</p></div><Button onClick={save} disabled={saving}>{saving ? <RefreshCw className="spin" size={15}/> : <Save size={15}/>} Save changes</Button></div>
      {message && <div className={`notice ${message.type}`}>{message.type === "ok" && <Check size={15}/>} {message.text}</div>}
      <div className="settings-layout">
        <div className="settings-main">
          <SettingsCard icon={<Power size={18}/>} title="Startup" description="Keep metrics collection and the local HTTP API available after you sign in.">
            <div className="setting-row startup-setting"><div><strong>Launch at startup</strong><small>Starts in the tray without opening the app window on macOS and Windows.</small></div><Switch label="Launch Computer State at startup" disabled={changingStartup} checked={startupEnabled} onChange={changeStartup}/></div>
          </SettingsCard>
          <SettingsCard icon={<Database size={18}/>} title="Collection & retention" description="Choose how frequently Computer State records a snapshot and how long it remains local.">
            <div className="field-grid">
              <label className="field"><span>Collection interval</span><small>Seconds between stored snapshots</small><input type="number" min={10} max={86400} value={settings.collection.interval_seconds} onChange={(e) => setCollection("interval_seconds", Number(e.target.value))}/></label>
              <label className="field"><span>History retention</span><small>Days before samples are deleted</small><input type="number" min={1} max={365} value={settings.collection.retention_days} onChange={(e) => setCollection("retention_days", Number(e.target.value))}/></label>
            </div>
            <p className="hint">Shortening retention permanently removes samples outside the new window during cleanup.</p>
          </SettingsCard>
          <SettingsCard icon={<Server size={18}/>} title="HTTP API" description="Configure the embedded Rocket server used by Prometheus and JSON clients.">
            <label className="field"><span>Listening port</span><small>Changes restart the local HTTP server</small><input type="number" min={1} max={65535} value={settings.http.port} onChange={(e) => setSettings({ ...settings, http: { ...settings.http, port: Number(e.target.value) } })}/></label>
            <div className="subsection"><span className="field-label">Allowed networks</span><small>Requests from other networks are rejected.</small>
              {[{ id: "loopback", label: "Localhost", detail: "127.0.0.1 and ::1" }, { id: "tailscale", label: "Tailscale", detail: "Your private tailnet" }].map((network) => <div className="setting-row" key={network.id}><div><strong>{network.label}</strong><small>{network.detail}</small></div><Switch label={`Allow ${network.label}`} checked={settings.http.allowed_interfaces.includes(network.id)} onChange={(checked) => setSettings({ ...settings, http: { ...settings.http, allowed_interfaces: checked ? [...settings.http.allowed_interfaces, network.id] : settings.http.allowed_interfaces.filter((item) => item !== network.id) } })}/></div>)}
              <label className="field custom-addresses"><span>Additional client addresses</span><small>Optional comma-separated IPv4 or IPv6 addresses</small><input type="text" placeholder="192.168.1.25, 10.0.0.4" value={settings.http.allowed_interfaces.filter((item) => item !== "loopback" && item !== "tailscale").join(", ")} onChange={(event) => { const standard = settings.http.allowed_interfaces.filter((item) => item === "loopback" || item === "tailscale"); const custom = event.target.value.split(",").map((item) => item.trim()).filter(Boolean); setSettings({ ...settings, http: { ...settings.http, allowed_interfaces: [...standard, ...custom] } }); }}/></label>
            </div>
          </SettingsCard>
          <SettingsCard icon={<FileText size={18}/>} title="Rocket server logs" description="Recent lifecycle and request activity from the embedded HTTP server.">
            <div className="log-toolbar"><span>{httpLogs.length} recent {httpLogs.length === 1 ? "entry" : "entries"}</span><Button variant="outline" onClick={refreshLogs} disabled={loadingLogs}><RefreshCw className={loadingLogs ? "spin" : undefined} size={14}/> Refresh</Button></div>
            <div className="server-logs" role="log" aria-label="Rocket server logs">
              {httpLogs.length === 0 ? <div className="empty-logs">No Rocket server activity yet.</div> : [...httpLogs].reverse().map((entry, index) => <div className="log-entry" key={`${entry.timestamp}-${index}`}><time dateTime={entry.timestamp}>{new Date(entry.timestamp).toLocaleString()}</time><strong className={entry.level === "ERROR" ? "log-error" : undefined}>{entry.level}</strong><code>{entry.message}</code></div>)}
            </div>
          </SettingsCard>
          <SettingsCard icon={<Globe2 size={18}/>} title="Exported metrics" description="Disabled metrics are no longer collected or exported. Existing history remains until it expires.">
            <div className="metric-settings">{catalog.map((metric) => <div className="setting-row" key={metric.id}><div><strong>{metric.label}</strong><small>{metric.available ? `${metric.id} · ${metric.unit}` : metric.reason}</small></div><Switch label={`Enable ${metric.label}`} disabled={!metric.available} checked={metric.available && settings.metrics[metric.id] !== false} onChange={(checked) => setSettings({ ...settings, metrics: { ...settings.metrics, [metric.id]: checked } })}/></div>)}</div>
          </SettingsCard>
        </div>
        <aside className="status-card card"><p className="eyebrow">Service status</p><h2>{status?.database_ready ? "Running" : "Starting"}<span className="status-dot"/></h2><dl><div><dt>API address</dt><dd>localhost:{status?.http_port ?? settings.http.port}</dd></div><div><dt>Database</dt><dd>{status?.database_ready ? "Connected" : "Unavailable"}</dd></div><div><dt>Latest sample</dt><dd>{status?.latest_collection ? new Date(status.latest_collection).toLocaleString() : "Waiting"}</dd></div><div><dt>Networks</dt><dd>{status?.allowed_interfaces.join(", ") ?? "—"}</dd></div></dl><div className="endpoint-list"><code>GET /metrics</code><code>GET /latest</code><code>GET /query</code></div></aside>
      </div>
    </section>
  );
}

function SettingsCard({ icon, title, description, children }: { icon: React.ReactNode; title: string; description: string; children: React.ReactNode }) {
  return <article className="card settings-card"><div className="settings-card-header"><div className="section-icon">{icon}</div><div><h2>{title}</h2><p>{description}</p></div></div><div className="settings-card-body">{children}</div></article>;
}
