# Computer State Architecture

This document defines the target architecture and development contract for Computer State. It describes how the application is expected to behave, how responsibilities are divided, and which constraints new code must preserve.

Computer State is currently based on the Tauri 2 React template. Several components described here have not yet been implemented or added as dependencies. This document is authoritative for the intended architecture; the source tree is authoritative for the current implementation state.

## 1. Goals

Computer State is a lightweight, local-first system monitoring application that:

- Runs as a tray application on macOS, Windows, and Linux
- Treats macOS and Windows as first-class, guaranteed platforms
- Supports Linux on a best-effort basis
- Collects enabled system metrics once per minute by default
- Stores seven days of metric history in SQLite by default
- Deletes expired samples automatically
- Presents stored metrics as customizable, automatically updating charts
- Exposes current metrics in Prometheus format
- Exposes current and historical metrics through a read-only JSON API
- Restricts HTTP access to localhost and Tailscale interfaces by default
- Stores all application data locally
- Uses as little CPU, memory, disk I/O, and network activity as practical while running in the background

The application is not intended to replace Prometheus or act as a long-term metrics database. SQLite provides a short local history for the desktop interface and JSON API; Prometheus remains responsible for its own scraping and retention.

## 2. Non-goals

The initial architecture does not include:

- Cloud synchronization or a hosted control plane
- User accounts or multi-user authorization
- Remote writes to Prometheus
- Unbounded local metric retention
- Arbitrary SQL or a general-purpose query language over the HTTP API
- A guarantee that every metric is available on every operating system or computer
- Perfect behavioral parity for Linux tray environments

## 3. Platform support

| Platform | Support level | Notes |
| --- | --- | --- |
| macOS | First-class | Menu-bar behavior, packaging, signing, and metric collection must be tested before release. |
| Windows | First-class | Notification-area behavior, packaging, signing, and metric collection must be tested before release. |
| Linux | Best effort | Availability depends on the distribution, GTK/AppIndicator integration, desktop environment, kernel, hardware, and drivers. |

Portable behavior must be implemented in shared code. Platform-specific collectors, tray behavior, icons, activation policies, and packaging belong behind narrow operating-system boundaries.

Code must not assume that GPU metrics, tray click events, tooltips, interface names, or a particular filesystem layout are universally available.

## 4. Technology stack

### Desktop shell and backend

- **Tauri 2** owns the native process, application lifecycle, tray icon, webview windows, and frontend command boundary.
- **Rust** owns metric collection, persistence, settings, HTTP serving, network policy, cleanup, and long-lived background tasks.
- **Tokio** is the asynchronous runtime used by Rocket and for the collector scheduler. Background work must not block Tauri's UI/event-loop thread.
- **Rocket** is the embedded HTTP framework. Rocket runs inside the Tauri process and serves the read-only HTTP API.
- **SQLx with SQLite** is the intended database layer. Migrations must be embedded in the application and applied at startup.
- **Serde/serde_json** define settings and API serialization.
- **sysinfo** should provide portable CPU, memory, process, storage, network, and uptime data where it has reliable coverage.
- GPU collection must use platform- or vendor-specific adapters behind a shared collector interface.

These backend libraries are target choices and must be added explicitly as implementation begins.

### Frontend

- **React** and **TypeScript** implement the webview interface.
- **React Router** owns navigation between `/metrics` and `/settings`.
- **shadcn/ui** supplies interface primitives.
- **shadcn/ui charts**, backed by Recharts, render metrics.
- CSS variables and shadcn theme tokens provide light and dark colors.
- The system color-scheme preference is the default and updates automatically while the application is running.

The frontend must not access SQLite, the settings file, operating-system APIs, or network sockets directly. It communicates with Rust through narrowly scoped Tauri commands and events.

## 5. Process and component model

Computer State runs as one native process containing the Tauri event loop, embedded HTTP server, scheduler, collectors, SQLite connection pool, and webview.

```text
Operating system
  |
  +-- Tauri process
      |
      +-- Tray controller
      +-- Window/router controller
      +-- Settings service ------ settings.json
      +-- Collection scheduler
      |   +-- Portable collectors
      |   +-- Platform collectors
      |   +-- Latest snapshot cache
      |   +-- Metric repository -- SQLite
      +-- Retention worker ------- SQLite
      +-- HTTP server
      |   +-- GET /metrics
      |   +-- GET /latest
      |   +-- GET /query
      +-- Tauri command boundary
          +-- React application
              +-- Metrics route
              +-- Settings route
```

Rust application state should contain handles to services rather than exposing raw database connections or mutable global values. A representative state container is:

```text
AppState
  settings_service
  metric_repository
  latest_snapshot
  collector_service
  http_server_control
```

Shared in-memory state must be thread-safe. Locks must not be held across filesystem, database, collection, or network awaits.

## 6. Startup and shutdown lifecycle

Startup order matters because the UI, collector, and HTTP server depend on validated settings and an initialized database.

1. Resolve and create the application data directory.
2. Load `settings.json`, validate it, and apply defaults for missing fields.
3. Open SQLite and apply embedded migrations.
4. Remove expired metric samples.
5. Initialize the latest-snapshot cache from the newest stored values, if present.
6. Start the collection scheduler.
7. Start HTTP listeners using the configured port and allowed interfaces.
8. Create the tray icon and its native menu.
9. Create the application window hidden or show it according to the startup policy.
10. Emit readiness to the frontend after backend services are available.

Normal window close requests hide the application window instead of terminating the process. The tray remains available and background collection continues.

The process exits only when the user selects **Quit** from the tray, the operating system terminates it, or startup encounters an unrecoverable failure. Explicit quit must stop accepting HTTP requests, finish or cancel active collection work, flush pending database operations, and then exit. Shutdown should be bounded so an unhealthy task cannot prevent exit indefinitely.

## 7. Background resource policy

Computer State is expected to remain running for long periods, so low background overhead is a primary architectural requirement rather than a later optimization.

When the application window is hidden, the steady-state process should do only the following:

- Keep the native tray icon and Rocket HTTP listener available
- Sleep until the next configured collection deadline
- Collect enabled metrics once
- Commit one bounded batch to SQLite
- Perform infrequent retention maintenance
- Respond to HTTP requests when they arrive

The hidden frontend must not poll for metrics, run chart animations, schedule refresh timers, or perform background rendering. Where supported by the webview, hiding the window should allow rendering and JavaScript activity to be suspended. Rust services remain authoritative and continue operating independently of frontend execution.

Background implementation rules:

- Use timers, events, and request-driven work; never use busy loops.
- Keep exactly one collection schedule and prevent overlapping collection runs.
- Refresh a subsystem once per collection cycle and derive all related metrics from that refresh.
- Collect only enabled metrics and avoid initializing expensive optional collectors, especially GPU collectors, until needed.
- Reuse the SQLite pool, Rocket server, collectors, and buffers instead of recreating them for each collection run.
- Batch each collection run's samples into one SQLite transaction.
- Keep database queries bounded and indexed.
- Do not poll network interfaces merely to determine whether a request arrived; let the operating system and Rocket listener block efficiently.
- Do not periodically scrape the application's own `/metrics` endpoint.
- Do not write routine successful collection details to disk logs every minute at normal log levels.
- Run cleanup at startup and no more frequently than required by the retention-maintenance schedule.
- Avoid full database vacuum operations during normal background activity.
- Release temporary collection buffers and platform handles promptly.
- Use bounded channels and caches so slow consumers cannot cause memory growth.

Performance must be measured in packaged release builds, not inferred from development mode. CI or release validation should record idle CPU, resident memory, wakeups, collection duration, database-write duration, and database size on macOS and Windows, with Linux measured when practical. Initial targets should be established from a working baseline and tightened before the first stable release; changes that materially increase idle resource use require an architectural justification.

## 8. Tray and window behavior

The tray menu contains exactly three primary actions:

1. **Metrics** — shows and focuses the application window at `/metrics`.
2. **Settings** — shows and focuses the application window at `/settings`.
3. **Quit** — performs a graceful shutdown and fully exits the process.

The first two actions reuse the existing window whenever possible. If the window was destroyed, Rust may recreate it before navigating. Opening a route must restore a minimized window and bring it to the foreground.

The Rust tray controller owns menu events. It sends a navigation event to the frontend or creates the window with the desired initial route. Tray behavior must not depend on frontend JavaScript being active while the window is hidden.

On macOS, the application should use menu-bar-appropriate behavior and artwork. On Windows it uses the notification area. Linux uses the available GTK/AppIndicator implementation and must rely on menu actions rather than custom tray click behavior.

## 9. Frontend architecture

### Routes and navigation

The application has two top-level routes:

| Route | View | Responsibility |
| --- | --- | --- |
| `/metrics` | Metrics | Display current and historical charts. |
| `/settings` | Settings | Configure collection, HTTP access, and appearance-independent application behavior. |

A tab navigation control at the top of the application mirrors these routes. URLs are the navigation source of truth, which allows tray actions and in-app navigation to select the same views consistently.

Unknown routes redirect to `/metrics`. Route components should be lazy-loadable, but shared application state and navigation remain above the route boundary.

### Metrics view

The Metrics view queries locally stored samples through Tauri commands, not through the public HTTP listener. This avoids coupling the desktop UI to listener configuration and ensures the UI still works if all external interfaces are disabled or the port cannot be exposed.

The view initially contains at least one chart. For each chart, the user can:

- Select an available metric from a dropdown
- Select a time range supported by the retained history
- Add another chart
- Remove a chart when more than one is present

There is no architectural limit on the number of charts, but the UI should virtualize or otherwise remain responsive with many charts. Requests for identical metric/range combinations should share cached data rather than issuing duplicate database queries.

Charts refresh when a new scheduled sample is committed. Rust emits a `metrics://sample-created` Tauri event after a successful collection transaction. The frontend invalidates relevant queries and redraws charts. A slow or hidden frontend must not block collection.

Charts use shadcn theme tokens. Metric colors must maintain sufficient contrast in both themes and must not be hard-coded solely for a light background.

### Settings view

The Settings view manages:

- Which metrics are enabled for collection and export
- The collection interval, defaulting to one minute
- The metric-history retention period, defaulting to seven days
- Which network interfaces or addresses may access the HTTP API
- The HTTP listening port, defaulting to `8888`
- Read-only application and API status information

Settings edits should use a draft form. Save validates the full document in Rust and applies it atomically. Invalid values are shown inline and do not partially update the running application.

Changing enabled metrics or the collection interval affects the next scheduled collection and subsequent exports. The scheduler must be rescheduled without creating an overlapping collection run. Disabling a metric does not delete its existing history; that history remains queryable until it expires.

Changing the retention period affects the next cleanup run. If the period is shortened, the settings UI must warn that the next cleanup permanently deletes samples newly outside the configured window. Increasing retention preserves future samples for longer but cannot restore data already deleted.

Changing the port or allowed interfaces requires a controlled HTTP server restart. The new settings file is committed only if the replacement listener can be established. If binding fails, the previous listener and previous settings remain active and the UI receives an actionable error.

### Theme behavior

Dark mode is supported from the first release. The root document follows `prefers-color-scheme` and listens for changes so switching the operating-system theme updates the open application without a restart.

The initial render must set the correct theme before displaying content to avoid a light-theme flash. All components and charts must be reviewed in both themes. The initial settings model does not need a manual theme override unless one is added deliberately later.

## 10. Application data layout

Tauri's application data directory is the only supported location for persistent application state. The exact base path is resolved with Tauri's path API and differs by operating system.

```text
<app-data-directory>/
  computer-state.sqlite3
  settings.json
  logs/                 # optional, if file logging is enabled
```

The settings file and SQLite database must always live in the same application data directory. Paths must never be built from hard-coded home-directory conventions.

The application must prevent secrets or private system data from being written to logs. Database and settings files should use permissions appropriate to the current user where the operating system supports them.

## 11. Settings model

`settings.json` is the source of truth for user configuration. A versioned structure allows migrations as fields evolve. The intended shape is:

```json
{
  "version": 1,
  "collection": {
    "interval_seconds": 60,
    "retention_days": 7
  },
  "http": {
    "port": 8888,
    "allowed_interfaces": ["loopback", "tailscale"]
  },
  "metrics": {
    "cpu_per_core_usage": true,
    "cpu_total_usage": true,
    "gpu_usage": true,
    "gpu_memory_total": true,
    "gpu_memory_usage": true,
    "memory_total": true,
    "memory_usage": true,
    "network_bytes_received": true,
    "network_bytes_sent": true,
    "process_count": true,
    "storage_total": true,
    "storage_usage": true,
    "uptime": true
  }
}
```

This example defines the initial schema, not a requirement to preserve these exact serialized names forever. Any changes must include a settings migration and corresponding documentation.

Settings rules:

- Missing settings create a default document.
- Missing fields receive defaults during migration.
- Unknown fields should be preserved when practical to avoid destructive downgrade behavior.
- The port must be in the inclusive range `1..=65535`.
- The collection interval and retention period must fall within documented, bounded ranges that prevent abusive scheduling or unbounded storage. The exact initial bounds must be finalized before implementation.
- Settings writes use a temporary file, flush, and atomic rename.
- Malformed settings are not silently overwritten. Preserve the invalid file, report the problem, and start with safe defaults only when recovery is possible.
- Frontend code receives a sanitized settings DTO through Tauri commands.

## 12. Metrics domain model

Metric identifiers are stable API contracts. They are independent of human-readable labels and database column names.

Initial canonical identifiers are:

| Identifier | Unit | Kind |
| --- | --- | --- |
| `cpu.core.usage` | percent | Gauge, one series per core |
| `cpu.total.usage` | percent | Gauge |
| `gpu.usage` | percent | Gauge, optionally one series per GPU |
| `gpu.memory.total` | bytes | Gauge, optionally one series per GPU |
| `gpu.memory.usage` | bytes | Gauge, optionally one series per GPU |
| `memory.total` | bytes | Gauge |
| `memory.usage` | bytes | Gauge |
| `network.bytes_received` | bytes | Monotonic counter, one series per interface |
| `network.bytes_sent` | bytes | Monotonic counter, one series per interface |
| `process.count` | processes | Gauge |
| `storage.total` | bytes | Gauge, one series per volume |
| `storage.usage` | bytes | Gauge, one series per volume |
| `uptime` | seconds | Gauge |

Each sample has:

- A canonical metric identifier
- A UTC timestamp
- A numeric value
- Zero or more bounded labels such as CPU core, GPU, network interface, or storage volume
- A unit known from metric metadata

Percent values use the range `0..=100`. Byte values are integer quantities at collection boundaries even if the database representation supports numeric aggregation. Timestamps are stored and exchanged in UTC. User-facing calendar dates are interpreted in the computer's current local timezone and converted to UTC query bounds.

Collectors return unavailable or unsupported states explicitly. They must not substitute zero for a failed or unsupported reading.

## 13. Collection pipeline

One scheduler triggers collection at the configured interval, which defaults to 60 seconds. Collection cadence should use a monotonic timer to avoid duplicate runs when the wall clock changes, while persisted timestamps use the current UTC wall clock.

For each run:

1. Snapshot the current enabled-metric settings.
2. Invoke collectors, allowing independent metric groups to run concurrently where safe.
3. Normalize values, units, identifiers, labels, and timestamps.
4. Record successful samples in one SQLite transaction.
5. Update the in-memory latest-snapshot cache only after the transaction commits.
6. Emit `metrics://sample-created` with the committed timestamp.
7. Log individual collector failures without discarding unrelated successful metrics.

Collection runs must not overlap. If a run exceeds the configured interval, skip or coalesce the next tick rather than accumulating an unbounded queue. Record enough structured logging to diagnose slow collectors.

The Prometheus endpoint performs or retrieves an instantaneous collection that is separate from the scheduled persistence pipeline. A Prometheus scrape must not write samples into SQLite or emit the sample-created event.

## 14. Collector interfaces

Collectors should implement a shared conceptual contract:

```text
Collector
  supported_metrics() -> MetricDescriptor[]
  collect(enabled_metrics) -> Result<MetricSample[], CollectorError>
```

Collectors should be grouped by subsystem rather than creating one operating-system call per metric. For example, one CPU refresh can generate total and per-core samples.

Implementation layers:

- Portable collector for CPU, memory, processes, storage, network, and uptime where supported
- macOS-specific adapters where portable APIs are insufficient
- Windows-specific adapters where portable APIs are insufficient
- Linux-specific adapters where portable APIs are insufficient
- An NVIDIA NVML adapter on Windows and Linux, selected at runtime when a compatible driver and GPU are present
- Explicit unavailable status for macOS and non-NVIDIA GPUs until another supported backend is added

Unsupported metrics remain visible to the settings and UI as unavailable with an explanation. They are omitted from `/latest` and `/metrics` until data becomes available.

## 15. SQLite design

SQLite is the source of truth for retained metric history and aggregation. It is not the source of truth for settings.

The normalized initial schema should be equivalent to:

```sql
CREATE TABLE metric_samples (
  id          INTEGER PRIMARY KEY,
  metric      TEXT NOT NULL,
  timestamp   INTEGER NOT NULL,
  value       REAL NOT NULL,
  labels_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX metric_samples_metric_timestamp
  ON metric_samples (metric, timestamp);

CREATE INDEX metric_samples_timestamp
  ON metric_samples (timestamp);
```

Timestamps should be stored as Unix milliseconds or another single documented integer resolution. The repository layer owns conversion; other layers must not depend on the physical representation.

SQLite should use WAL mode and a short busy timeout. Database access must be performed through a bounded pool. Queries must use bound parameters and an allowlist of metric identifiers and aggregation operations.

All schema changes require numbered, forward-only migrations. CI must test both an empty database and migration from every supported prior application schema.

## 16. Retention and cleanup

The retention period is configurable and defaults to seven days. Samples older than the current UTC time minus the configured retention period are deleted automatically.

Cleanup runs:

- During startup, before normal queries begin
- At least once every 24 hours while the application remains running, and after a retention-setting change

Cleanup uses a bounded delete operation and must not hold a UI or service-wide lock. SQLite page reclamation should not run a full blocking `VACUUM` on every cleanup. Checkpointing or vacuum behavior should be measured and scheduled separately if database growth requires it.

Expired history is permanently unavailable to `/query` and the Metrics view. Disabling a metric is not a reason to delete its unexpired samples. Cleanup always reads the current configured retention period rather than assuming seven days.

## 17. Repository and query behavior

All SQLite access goes through a metric repository. Required operations include:

- Insert a batch of samples transactionally
- Fetch the latest value for every requested metric and label set
- Fetch raw samples over a bounded range
- Aggregate samples over a bounded range
- Aggregate samples into fixed intervals
- Delete samples older than a cutoff

The repository must enforce the configured retention boundary even if a caller requests an earlier start time. Raw responses need pagination or a hard result limit so a request cannot allocate unbounded memory. Aggregate queries should be preferred for chart ranges that would otherwise return excessive points.

Supported initial aggregations are `avg`, `min`, `max`, `sum`, and `count`. Dynamic SQL fragments are selected from a fixed server-side mapping; they are never copied directly from request text.

## 18. HTTP API

The embedded HTTP API is implemented with Rocket, is read-only, and listens on port `8888` by default. A settings change may select another valid port. Rocket must be launched and shut down through application lifecycle management rather than as an independent sidecar process.

Rocket routes are thin adapters over the same metric and query services used by Tauri commands. Managed Rocket state may hold cloneable service handles, but route handlers must not contain collector, SQL, retention, or settings business logic. Request guards or fairings enforce network policy, correlation IDs, safe error mapping, and response headers consistently.

### Network policy

Default listeners are limited to:

- IPv4 and IPv6 loopback
- Detected Tailscale addresses/interfaces

The settings view may add or remove allowed interfaces. The implementation must bind only to selected local addresses where possible and also validate the remote peer against the effective policy. It must not silently fall back to `0.0.0.0` or `::` when a requested interface cannot be resolved.

Because a Rocket instance is configured with one listening address, the HTTP listener manager may launch one Rocket instance per selected local address while sharing the same application services. The manager owns this listener set as one logical server, detects duplicate addresses, coordinates graceful shutdown, and reports partial bind failures. A wildcard-bound Rocket instance is acceptable only if a future implementation proves that application-layer peer filtering is reliable on every supported platform and the user explicitly selected equivalent exposure.

CORS is disabled by default. Enabling browser access from arbitrary origins requires an explicit future security design. Requests and errors must not disclose filesystem paths or internal SQL.

### `GET /metrics`

Returns an instantaneous snapshot in the Prometheus text exposition format.

- Includes enabled and currently available metrics
- Does not query historical SQLite data
- Does not store its snapshot in SQLite
- Uses valid Prometheus metric and label names derived from canonical identifiers
- Returns an error if an instantaneous snapshot cannot be produced at all; partial collector availability may be represented by omitted series and internal health logging

### `GET /latest`

Returns the newest persisted sample for each enabled and available metric and label set. It reads the latest-snapshot cache, which is initialized from SQLite and updated only after successful collection commits.

### `GET /query`

Queries persisted SQLite history. Supported parameters are:

| Parameter | Behavior |
| --- | --- |
| `metric` | Required and repeatable canonical identifier. |
| `from` | Inclusive RFC 3339 start timestamp. |
| `to` | Exclusive RFC 3339 end timestamp; defaults to now. |
| `date` | A local calendar date in `YYYY-MM-DD`. |
| `range` | A relative range such as `30m`, `6h`, `2d`, or `1w`. |
| `aggregation` | `avg`, `min`, `max`, `sum`, or `count`. |
| `interval` | Optional aggregation bucket such as `5m` or `1h`. |

`date`, `range`, and `from` are mutually exclusive query modes. `from` can be paired with `to`. `range` ends at `to` when supplied, otherwise it ends at the current time.

For example, average total CPU usage over the last six hours is:

```text
GET /query?metric=cpu.total.usage&range=6h&aggregation=avg
```

Input parsing must reject unknown metrics, invalid durations, inverted ranges, unsupported aggregations, intervals without aggregation, and combinations that are ambiguous. API DTOs are separate from database and collector types so schemas can evolve intentionally.

### HTTP responses

- `/metrics` uses the Prometheus text content type.
- `/latest` and `/query` use `application/json`.
- `200` indicates a successful request, including a valid query with no samples.
- `400` indicates invalid parameters.
- `403` indicates a request rejected by network policy when rejection occurs at the application layer.
- `500` indicates an unexpected internal failure without exposing sensitive details.
- `503` indicates that a required service or snapshot is temporarily unavailable.

Every request should have a correlation ID in structured logs. Avoid logging full query responses or high-cardinality metric labels by default.

## 19. Tauri command and event boundary

The frontend uses Tauri commands for local application operations. The intended command surface includes:

```text
get_settings
update_settings
get_metric_catalog
query_metric_history
get_latest_metrics
get_service_status
```

Command names and DTOs are API contracts. Commands validate all inputs in Rust even when the frontend has already validated them.

Events emitted by Rust include:

```text
metrics://sample-created
settings://updated
service://status-changed
navigation://requested
```

Event payloads should be small notifications or summaries, not full metric histories. The frontend fetches authoritative data after invalidation.

Tauri capabilities must grant only the APIs the application uses. Do not enable broad filesystem, shell, or network access for frontend JavaScript.

## 20. Error handling and observability

Errors should be typed by boundary:

- Collector errors identify the metric group and whether failure is unsupported, unavailable, permission-related, or transient.
- Repository errors wrap database operations without leaking SQL to clients.
- Settings errors distinguish validation, migration, parsing, and persistence failures.
- HTTP errors map internal errors to stable status codes and safe response bodies.

One failed collector must not stop unrelated collectors, the HTTP server, or the UI. Repeated background failures should use rate-limited logging.

Structured logs should include timestamps, severity, component, and relevant operation IDs. Logs belong in the application data directory when file logging is enabled. Metric values should not normally be logged.

The Settings view should surface service health such as the last successful collection, database status, HTTP listener addresses, configured port, and unavailable metrics.

## 21. Security and privacy

Security defaults are restrictive:

- Do not bind to public interfaces unless the user explicitly selects them.
- Keep the HTTP API read-only.
- Validate peer addresses and every query parameter.
- Use bound SQL parameters and server-controlled aggregation expressions.
- Do not expose arbitrary files, SQL, commands, or operating-system process details through the API.
- Do not treat interface names supplied by the frontend as trusted.
- Use least-privilege Tauri capabilities.
- Keep settings and metric history local to the current user's application data directory.

The API does not initially include authentication. Consequently, expanding access beyond localhost and Tailscale must be presented as a security-sensitive setting. Authentication and TLS require a separate design before exposing the service to untrusted networks.

## 22. Testing strategy

### Rust unit tests

- Settings defaults, validation, migrations, and atomic update behavior
- Metric normalization and unit conversion
- Duration and RFC 3339 parsing
- Query validation and aggregation selection
- Retention cutoff calculations
- Collection interval rescheduling and bounds
- Network-interface policy evaluation
- Prometheus name and label conversion

### Repository integration tests

- Migrations on empty and prior-schema databases
- Transactional batch insertion
- Latest-value selection per label set
- Raw, aggregate, and bucketed queries
- Configured retention-boundary behavior, including the seven-day default
- Cleanup without deleting newer samples
- Concurrent reads during collection and cleanup

Use temporary directories and SQLite databases. Tests must not use or modify the developer's real application data.

### HTTP integration tests

- Content types and representative successful responses
- All valid query modes
- Multiple `metric` parameters
- Average CPU over a six-hour relative range
- Empty ranges
- Invalid and conflicting parameters
- Network-policy rejection
- Result limits and cancellation

### Frontend tests

- Route and top-tab synchronization
- Tray navigation event handling
- Adding, removing, and changing charts
- Refresh after `metrics://sample-created`
- Settings validation and failed server-restart recovery
- Live system-theme changes
- Loading, empty, unavailable, and error states

### Platform tests

Release candidates must be exercised on supported macOS and Windows versions. Linux testing should cover at least one GNOME/AppIndicator environment and one KDE environment when practical. Platform tests include tray actions, close-to-tray behavior, quit, startup, filesystem paths, interface detection, HTTP binding, and available collectors.

### Background performance tests

Packaged release builds must be profiled with the application window hidden. Tests should cover idle operation between collection ticks, a collection and SQLite commit, an HTTP request, and retention cleanup. At minimum, record CPU use, resident memory, process wakeups, collection latency, database-write latency, and unexpected network or disk activity. Frontend timers and chart work must stop while the window is hidden.

## 23. Source organization

The target source layout should make boundaries visible:

```text
src/
  app/
    router.tsx
    providers.tsx
  components/
    ui/                    # generated/customized shadcn components
    charts/
    layout/
  features/
    metrics/
    settings/
  lib/
    tauri.ts
    theme.ts
  types/

src-tauri/src/
  lib.rs
  app_state.rs
  tray.rs
  commands/
  collectors/
    mod.rs
    portable.rs
    macos.rs
    windows.rs
    linux.rs
    gpu/
  database/
    mod.rs
    repository.rs
    migrations/
  metrics/
    model.rs
    catalog.rs
    scheduler.rs
    retention.rs
  http/
    mod.rs
    routes.rs
    query.rs
    prometheus.rs
    policy.rs
  settings/
    mod.rs
    model.rs
    migration.rs
  error.rs
```

This is directional rather than a demand for empty modules. Create boundaries as functionality is implemented, and avoid generic `utils` modules that conceal ownership.

## 24. Development rules

- Rust owns business logic and persistent state; React owns presentation and user interaction.
- SQLite is the only historical metric store.
- `settings.json` is the only persistent settings store.
- Public HTTP handlers and Tauri commands call shared services rather than duplicating query or validation logic.
- Prometheus scrapes never create SQLite history.
- Only the configured collection scheduler creates retained samples.
- The frontend never depends on the public HTTP server to function.
- All timestamps crossing a backend boundary use RFC 3339 UTC strings.
- All byte quantities use bytes, durations use seconds unless explicitly identified otherwise, and percentages use `0..=100`.
- New metrics require a catalog entry, collector support declaration, settings migration/default, Prometheus mapping, API documentation, chart metadata, and tests.
- New settings require validation, migration behavior, atomic persistence, UI handling, and tests.
- Platform-specific code must have a documented fallback or explicit unavailable state.
- Background features must be event-driven, bounded, and included in resource profiling.
- A hidden window must not continue dashboard polling, animation, or rendering work.
- A release is not complete until tray lifecycle, retention, API binding, and automatic theme behavior have been tested on its target platforms.

## 25. Current implementation decisions

- Exact dependency versions are locked by `Cargo.lock` and `bun.lock`.
- The application window is shown at startup and hides instead of closing. Tray actions restore it to the requested route.
- Raw queries return at most 50,000 samples in total across no more than 16 requested metrics. Additional pagination can be introduced without removing this safety bound.
- Raw and aggregate queries both return label-aware `MetricSample` objects in a `samples` array.
- Dashboard chart layout is session-local and is not persisted across restarts.
- NVIDIA GPU utilization and total/used VRAM are collected through NVML on Windows and Linux. GPU metrics remain in the catalog but report unavailable on macOS, non-NVIDIA hardware, or hosts without a compatible NVIDIA driver.
- Launch-at-login is not enabled by the initial implementation.
- File logging is disabled by default; normal operation writes no per-sample log file.

Future decisions must not weaken the security, retention, ownership, background-resource, or platform boundaries defined in this document.
