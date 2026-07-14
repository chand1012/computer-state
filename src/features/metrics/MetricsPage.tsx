import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Area, AreaChart, CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";
import { Activity, AlertCircle, Plus, Trash2 } from "lucide-react";
import { api, formatMetric, type MetricDescriptor, type MetricSample } from "../../lib/api";
import { Button } from "../../components/ui/Button";
import { ChartContainer } from "../../components/ui/Chart";

export type ChartConfig = { id: string; metric: string; hours: number };
type TimeBounds = { from: number; to: number };
const CHART_STORAGE_KEY = "computer-state.metrics.charts.v1";
const ranges = [
  { label: "15 minutes", hours: 0.25, intervalSeconds: 15 },
  { label: "1 hour", hours: 1, intervalSeconds: 60 },
  { label: "6 hours", hours: 6, intervalSeconds: 300 },
  { label: "24 hours", hours: 24, intervalSeconds: 900 },
  { label: "7 days", hours: 168, intervalSeconds: 3600 },
];

export function MetricsPage() {
  const [catalog, setCatalog] = useState<MetricDescriptor[]>([]);
  const [charts, setCharts] = useState<ChartConfig[]>(readStoredCharts);

  useEffect(() => { api.catalog().then(setCatalog); }, []);
  useEffect(() => {
    try { localStorage.setItem(CHART_STORAGE_KEY, JSON.stringify(charts)); } catch { /* Persistence may be unavailable in restricted webviews. */ }
  }, [charts]);
  const available = catalog.filter((metric) => metric.available);

  return (
    <section>
      <div className="page-heading">
        <div><p className="eyebrow">Live history</p><h1>System metrics</h1><p>Explore the metrics stored locally on this computer.</p></div>
        <Button onClick={() => setCharts((items) => [...items, { id: crypto.randomUUID(), metric: available[0]?.id ?? "cpu.total.usage", hours: 6 }])}><Plus size={15} /> Add chart</Button>
      </div>
      <div className="chart-grid">
        {charts.map((chart) => (
          <MetricChart key={chart.id} config={chart} catalog={catalog} canRemove={charts.length > 1}
            onChange={(next) => setCharts((items) => items.map((item) => item.id === chart.id ? next : item))}
            onRemove={() => setCharts((items) => items.filter((item) => item.id !== chart.id))} />
        ))}
      </div>
    </section>
  );
}

function defaultCharts(): ChartConfig[] {
  return [{ id: crypto.randomUUID(), metric: "cpu.total.usage", hours: 6 }];
}

function readStoredCharts(): ChartConfig[] {
  try { return parseStoredCharts(localStorage.getItem(CHART_STORAGE_KEY)) ?? defaultCharts(); }
  catch { return defaultCharts(); }
}

export function parseStoredCharts(value: string | null): ChartConfig[] | undefined {
  if (!value) return undefined;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed) || parsed.length === 0) return undefined;
    const charts = parsed.filter((item): item is ChartConfig => {
      if (!item || typeof item !== "object") return false;
      const chart = item as Partial<ChartConfig>;
      return typeof chart.id === "string" && chart.id.length > 0
        && typeof chart.metric === "string" && chart.metric.length > 0
        && ranges.some((range) => range.hours === chart.hours);
    });
    return charts.length === parsed.length ? charts : undefined;
  } catch { return undefined; }
}

function MetricChart({ config, catalog, canRemove, onChange, onRemove }: { config: ChartConfig; catalog: MetricDescriptor[]; canRemove: boolean; onChange: (value: ChartConfig) => void; onRemove: () => void }) {
  const [samples, setSamples] = useState<MetricSample[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [updatedAt, setUpdatedAt] = useState<Date>();
  const [timeBounds, setTimeBounds] = useState<TimeBounds>(() => createTimeBounds(config.hours));
  const descriptor = catalog.find((metric) => metric.id === config.metric);

  const load = useCallback(async () => {
    try {
      setError("");
      const bounds = createTimeBounds(config.hours);
      const range = ranges.find((item) => item.hours === config.hours) ?? ranges[2];
      setSamples(await api.history([config.metric], new Date(bounds.from), new Date(bounds.to), "avg", range.intervalSeconds));
      setTimeBounds(bounds);
      setUpdatedAt(new Date());
    } catch (cause) { setError(String(cause)); } finally { setLoading(false); }
  }, [config.metric, config.hours]);

  useEffect(() => { setLoading(true); load(); }, [load]);
  useEffect(() => {
    let stop: undefined | (() => void); let disposed = false;
    listen("metrics://sample-created", () => { if (document.visibilityState === "visible") load(); }).then((fn) => disposed ? fn() : stop = fn);
    const visible = () => { if (document.visibilityState === "visible") load(); };
    document.addEventListener("visibilitychange", visible);
    return () => { disposed = true; stop?.(); document.removeEventListener("visibilitychange", visible); };
  }, [load]);

  const { data, series } = useMemo(() => buildChartData(samples), [samples]);
  const chartData = useMemo(() => [{ timestamp: timeBounds.from }, ...data, { timestamp: timeBounds.to }], [data, timeBounds]);
  const latest = samples.reduce<MetricSample | undefined>((newest, sample) => !newest || sample.timestamp > newest.timestamp ? sample : newest, undefined)?.value;

  return (
    <article className="card chart-card">
      <div className="chart-toolbar">
        <div className="select-group">
          <select aria-label="Metric" value={config.metric} onChange={(event) => onChange({ ...config, metric: event.target.value })}>
            {catalog.map((metric) => <option key={metric.id} value={metric.id} disabled={!metric.available}>{metric.label}{metric.available ? "" : " — unavailable"}</option>)}
          </select>
          <select aria-label="Time range" value={config.hours} onChange={(event) => onChange({ ...config, hours: Number(event.target.value) })}>
            {ranges.map((range) => <option key={range.hours} value={range.hours}>{range.label}</option>)}
          </select>
        </div>
        {canRemove && <Button variant="ghost" aria-label="Remove chart" onClick={onRemove}><Trash2 size={16} /></Button>}
      </div>
      <div className="chart-summary"><span>{descriptor?.label ?? config.metric}</span><strong>{latest === undefined || !descriptor ? "—" : formatMetric(latest, descriptor.unit)}</strong><small className="live-status"><i />Live{updatedAt ? ` · Updated ${updatedAt.toLocaleTimeString([], { hour: "numeric", minute: "2-digit", second: "2-digit" })}` : ""}</small></div>
      <div className="chart-area">
        {loading ? <div className="chart-state"><div className="spinner" /> Loading history…</div> : error ? <div className="chart-state error"><AlertCircle size={19} />{error}</div> : data.length === 0 ? <div className="chart-state"><Activity size={20} />Waiting for the first stored sample…</div> : (
          <ChartContainer>
            <AreaChart data={chartData} margin={{ top: 8, right: 12, left: 0, bottom: 0 }}>
              <defs>{series.map((key, index) => <linearGradient key={key} id={`fill-${config.id}-${index}`} x1="0" y1="0" x2="0" y2="1"><stop offset="5%" stopColor={`var(--chart-${index % 5 + 1})`} stopOpacity={0.28}/><stop offset="95%" stopColor={`var(--chart-${index % 5 + 1})`} stopOpacity={0}/></linearGradient>)}</defs>
              <CartesianGrid strokeDasharray="3 3" vertical={false} stroke="var(--border)" />
              <XAxis dataKey="timestamp" type="number" scale="linear" domain={[timeBounds.from, timeBounds.to]} allowDataOverflow tickCount={6} tickFormatter={(value) => formatAxisTime(Number(value), config.hours)} tick={{ fill: "var(--muted-foreground)", fontSize: 11 }} axisLine={false} tickLine={false} minTickGap={28} />
              <YAxis tickFormatter={(value) => descriptor ? formatMetric(value, descriptor.unit).replace(/\s/g, " ") : value} width={62} tick={{ fill: "var(--muted-foreground)", fontSize: 11 }} axisLine={false} tickLine={false} />
              <Tooltip contentStyle={{ background: "var(--popover)", border: "1px solid var(--border)", borderRadius: 10, color: "var(--popover-foreground)" }} labelFormatter={(value) => new Date(value).toLocaleString()} formatter={(value) => descriptor ? formatMetric(Number(value), descriptor.unit) : value} />
              {series.map((key, index) => <Area key={key} type="monotone" dataKey={key} stroke={`var(--chart-${index % 5 + 1})`} strokeWidth={2} fill={`url(#fill-${config.id}-${index})`} dot={data.length <= 1 ? { r: 3, fill: `var(--chart-${index % 5 + 1})`, stroke: "var(--card)", strokeWidth: 2 } : false} connectNulls isAnimationActive={false} />)}
            </AreaChart>
          </ChartContainer>
        )}
      </div>
    </article>
  );
}

export function buildChartData(samples: MetricSample[]) {
  const series = Array.from(new Set(samples.map((sample) => Object.values(sample.labels).join(" · ") || "value")));
  const byTime = new Map<number, Record<string, number>>();
  for (const sample of samples) {
    const key = Object.values(sample.labels).join(" · ") || "value";
    byTime.set(sample.timestamp, { ...(byTime.get(sample.timestamp) ?? {}), [key]: sample.value });
  }
  return { series, data: Array.from(byTime, ([timestamp, values]) => ({ timestamp, ...values })).sort((a, b) => a.timestamp - b.timestamp) };
}

export function createTimeBounds(hours: number, now = Date.now()): TimeBounds {
  return { from: now - hours * 3_600_000, to: now };
}

function formatAxisTime(timestamp: number, hours: number) {
  const date = new Date(timestamp);
  if (hours > 24) return date.toLocaleDateString([], { month: "short", day: "numeric" });
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}
