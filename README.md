# Computer State

Computer State is a lightweight system monitoring application for macOS, Windows, and Linux. It runs in the system tray and exposes system metrics through both a Prometheus-compatible endpoint and a JSON REST API.

The application is designed for monitoring computers that may not otherwise run a full observability agent. It provides a real-time view of current and recent system information while keeping collection, configuration, and historical data local to the computer.

## How it works

Computer State samples enabled system metrics once per minute by default. Samples are stored in a local SQLite database, giving the JSON API access to current and recent historical data without requiring an external database. The collection interval and history-retention period are configurable; by default, the application automatically deletes samples older than seven days.

Prometheus metrics follow the standard pull model. When Prometheus scrapes Computer State, the application collects or reads the current values and returns them in Prometheus format. Prometheus samples are not written to Computer State's SQLite history; Prometheus is responsible for storing and retaining the data it scrapes.

At a high level, Computer State provides:

- A cross-platform tray application
- Metric collection once per minute by default
- Seven days of local history in SQLite by default
- A JSON REST API for current and historical metrics
- A Prometheus-compatible endpoint for current metrics
- A real-time, customizable dashboard for locally stored metric history
- Controls for enabling or disabling individual metrics
- Controls for choosing which network interfaces may access the API

## Metrics

Computer State can export the following system metrics:

- CPU utilization per core
- Total CPU utilization
- GPU utilization
- Total GPU memory
- GPU memory utilization
- Total system memory
- System memory utilization
- Network bytes received
- Network bytes sent
- Number of running processes
- Total storage capacity
- Storage utilization
- System uptime

Metric availability may vary by operating system and hardware. NVIDIA GPU utilization and memory telemetry are supported on Windows and Linux through NVML when a compatible driver is installed. Other GPU vendors and macOS report GPU metrics as unavailable until a supported telemetry backend is present.

## API access and security

The HTTP API defaults to local access only. Computer State permits connections from localhost and Tailscale networks by default and does not expose metrics broadly to the local network or internet.

Allowed network interfaces can be changed from the application settings. This makes it possible to expose Computer State to a trusted network when needed while maintaining a restrictive default configuration.

## HTTP API

Computer State exposes three read-only HTTP endpoints and listens on port `8888` by default. The listening port can be changed from the application settings.

### `GET /metrics`

Returns the current values of all enabled metrics in the Prometheus exposition format. Configure a Prometheus server to scrape this endpoint:

```yaml
scrape_configs:
  - job_name: computer-state
    static_configs:
      - targets:
          - localhost:8888
```

A direct request can also be made with curl:

```sh
curl http://localhost:8888/metrics
```

The response contains only the current metric snapshot. Serving this endpoint does not add another sample to SQLite, and Computer State does not retain Prometheus scrape responses. Prometheus is responsible for storing and retaining the samples it collects.

### `GET /latest`

Returns the latest locally collected values for all enabled metrics as JSON:

```sh
curl http://localhost:8888/latest
```

The response includes the collection timestamp and the latest value of each available metric:

```json
{
  "timestamp": "2026-07-12T14:30:00Z",
  "metrics": [
    {
      "metric": "cpu.total.usage",
      "timestamp": 1783866600000,
      "value": 24.8,
      "labels": {}
    },
    {
      "metric": "cpu.core.usage",
      "timestamp": 1783866600000,
      "value": 31.2,
      "labels": { "core": "0" }
    }
  ]
}
```

Metric values that are disabled, unsupported, or unavailable on the current computer are omitted.

### `GET /query`

Queries metrics stored in SQLite. A query can request one or more metrics over an explicit date range, a specific calendar date, or a relative time frame such as the last six hours.

Supported query parameters are:

| Parameter | Description |
| --- | --- |
| `metric` | Metric name to return. Repeat the parameter to request multiple metrics. |
| `from` | Inclusive start timestamp in RFC 3339 format. |
| `to` | Exclusive end timestamp in RFC 3339 format. Defaults to the current time when omitted. |
| `date` | Calendar date in `YYYY-MM-DD` format. Queries that entire local calendar day. |
| `range` | Relative time frame ending at `to` or the current time, such as `30m`, `6h`, `2d`, or `1w`. The requested range cannot exceed retained history. |
| `aggregation` | Optional aggregation: `avg`, `min`, `max`, `sum`, or `count`. |
| `interval` | Optional time bucket for aggregated results, such as `5m` or `1h`. If omitted, aggregation produces one value for the complete range. |

`date`, `range`, and `from` describe alternative ways to select the beginning of a query. A request should use only one of these modes. `from` may be combined with `to`, while `range` may use `to` to select a relative window ending at a historical time.

#### Query raw samples

Get total CPU utilization samples from an explicit period:

```sh
curl "http://localhost:8888/query?metric=cpu.total.usage&from=2026-07-12T08:00:00Z&to=2026-07-12T12:00:00Z"
```

Get CPU and memory samples from a local calendar date:

```sh
curl "http://localhost:8888/query?metric=cpu.total.usage&metric=memory.usage&date=2026-07-12"
```

Get all stored samples for a metric from the last 30 minutes:

```sh
curl "http://localhost:8888/query?metric=network.bytes_received&range=30m"
```

#### Query aggregate values

Get the average total CPU utilization over the last six hours:

```sh
curl "http://localhost:8888/query?metric=cpu.total.usage&range=6h&aggregation=avg"
```

This returns one aggregate sample for each label set over the complete six-hour range:

```json
{
  "from": "2026-07-12T08:30:00Z",
  "to": "2026-07-12T14:30:00Z",
  "aggregation": "avg",
  "interval": null,
  "samples": [
    {
      "metric": "cpu.total.usage",
      "timestamp": 1783845000000,
      "value": 31.4,
      "labels": {}
    }
  ]
}
```

Add an interval to return aggregated time buckets. For example, this request returns an average CPU value for each five-minute period during the last six hours:

```sh
curl "http://localhost:8888/query?metric=cpu.total.usage&range=6h&aggregation=avg&interval=5m"
```

Raw queries return timestamped samples. Bucketed aggregate queries return timestamped buckets. Queries are restricted to the history currently retained on the computer, so they cannot return samples that have already expired.

### API responses

The JSON endpoints use `application/json`, while `/metrics` uses the Prometheus text content type. Successful requests return HTTP `200`. Invalid metric names, time ranges, parameter combinations, or aggregation options return HTTP `400` with a JSON error description. Requests from interfaces that are not allowed by the application settings are rejected.

The application automatically deletes stored metric samples when they exceed the configured retention period, which defaults to seven days.

## Application interface

Computer State runs primarily as a background process in the system tray. The tray menu provides quick access to the application, its status, and common actions.

The application window contains two views, available from a tab menu at the top of the page:

### Dashboard

The dashboard displays real-time graphs based on current and locally stored historical system metrics. Users can select a metric from a simple dropdown menu and switch the graph between CPU, memory, network, storage, and other available system data. Time ranges begin at 15 minutes.

The dashboard can display multiple graphs at the same time. Users may add as many metric graphs as they need, allowing them to monitor several aspects of the computer from a single view. Graphs use a Grafana-style time axis: older samples move left while the newest data remains at the right edge and updates as new samples are collected.

Charts are built with the shadcn/ui chart components and automatically follow the operating system's light or dark theme.

### Settings

The settings view controls metric collection and API access. From this view, users can:

- Enable or disable individual exported metrics
- Change the metric collection interval from its default of one minute
- Change the metric-history retention period from its default of seven days
- Configure which network interfaces may access the HTTP API
- Change the HTTP API's listening port from its default of `8888`
- Review application and API settings

The interface is built with shadcn/ui and supports both light and dark themes. The entire application, including its charts, follows the operating system setting automatically.

## Platform support

macOS and Windows are first-class platforms. Linux is supported as a secondary platform, subject to the system tray and metric APIs provided by the installed desktop environment, kernel, hardware, and drivers.

## Data ownership

Computer State is local-first:

- Metric history remains in a local SQLite database.
- Historical samples older than the configured retention period are deleted automatically from SQLite; the default retention period is seven days.
- Prometheus data is sent only in response to a scrape and is retained by the Prometheus server.
- API exposure is restricted to localhost and Tailscale networks by default.

## Project status

Computer State has a working cross-platform Tauri implementation. Platform-specific metric availability—particularly GPU telemetry—depends on the host hardware, drivers, and operating-system APIs. See `ARCHITECTURE.md` for implementation details and development constraints.
