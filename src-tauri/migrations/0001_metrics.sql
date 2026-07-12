CREATE TABLE IF NOT EXISTS metric_samples (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  metric      TEXT NOT NULL,
  timestamp   INTEGER NOT NULL,
  value       REAL NOT NULL,
  labels_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS metric_samples_metric_timestamp
  ON metric_samples (metric, timestamp);

CREATE INDEX IF NOT EXISTS metric_samples_timestamp
  ON metric_samples (timestamp);
