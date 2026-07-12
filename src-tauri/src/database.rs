use crate::model::MetricSample;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::{collections::BTreeMap, path::Path};

pub struct MetricRepository {
    pool: SqlitePool,
}

impl MetricRepository {
    pub async fn open(path: &Path) -> Result<Self, String> {
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .map_err(|error| format!("failed to open metrics database: {error}"))?;
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|error| format!("failed to migrate database: {error}"))?;
        Ok(Self { pool })
    }

    pub async fn insert(&self, samples: &[MetricSample]) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        for sample in samples {
            sqlx::query(
                "INSERT INTO metric_samples(metric,timestamp,value,labels_json) VALUES(?,?,?,?)",
            )
            .bind(&sample.metric)
            .bind(sample.timestamp)
            .bind(sample.value)
            .bind(serde_json::to_string(&sample.labels).map_err(|e| e.to_string())?)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())
    }

    pub async fn latest(&self, metrics: &[String]) -> Result<Vec<MetricSample>, String> {
        let mut output = Vec::new();
        for metric in metrics {
            let rows = sqlx::query(
                "SELECT metric,timestamp,value,labels_json FROM metric_samples m WHERE metric=? AND timestamp=(SELECT MAX(timestamp) FROM metric_samples WHERE metric=m.metric) ORDER BY labels_json",
            )
            .bind(metric)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            for row in rows {
                output.push(row_to_sample(&row)?);
            }
        }
        Ok(output)
    }

    pub async fn raw(
        &self,
        metrics: &[String],
        from: i64,
        to: i64,
        limit: i64,
    ) -> Result<Vec<MetricSample>, String> {
        let mut output = Vec::new();
        for metric in metrics {
            let rows = sqlx::query("SELECT metric,timestamp,value,labels_json FROM metric_samples WHERE metric=? AND timestamp>=? AND timestamp<? ORDER BY timestamp ASC LIMIT ?")
                .bind(metric).bind(from).bind(to).bind(limit)
                .fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
            for row in rows {
                output.push(row_to_sample(&row)?);
            }
        }
        output.sort_by_key(|sample| sample.timestamp);
        Ok(output)
    }

    pub async fn aggregate(
        &self,
        metrics: &[String],
        from: i64,
        to: i64,
        operation: &str,
        interval_ms: Option<i64>,
    ) -> Result<Vec<MetricSample>, String> {
        let sql_op = match operation {
            "avg" => "AVG",
            "min" => "MIN",
            "max" => "MAX",
            "sum" => "SUM",
            "count" => "COUNT",
            _ => return Err("unsupported aggregation".into()),
        };
        let mut output = Vec::new();
        for metric in metrics {
            let sql = if interval_ms.is_some() {
                format!("SELECT metric,(timestamp / ?) * ? AS timestamp,{sql_op}(value) AS value,labels_json FROM metric_samples WHERE metric=? AND timestamp>=? AND timestamp<? GROUP BY metric,labels_json,(timestamp / ?) ORDER BY timestamp")
            } else {
                format!("SELECT metric,? AS timestamp,{sql_op}(value) AS value,labels_json FROM metric_samples WHERE metric=? AND timestamp>=? AND timestamp<? GROUP BY metric,labels_json")
            };
            let rows = if let Some(interval) = interval_ms {
                sqlx::query(&sql)
                    .bind(interval)
                    .bind(interval)
                    .bind(metric)
                    .bind(from)
                    .bind(to)
                    .bind(interval)
                    .fetch_all(&self.pool)
                    .await
            } else {
                sqlx::query(&sql)
                    .bind(from)
                    .bind(metric)
                    .bind(from)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await
            }
            .map_err(|e| e.to_string())?;
            for row in rows {
                output.push(row_to_sample(&row)?);
            }
        }
        Ok(output)
    }

    pub async fn cleanup(&self, cutoff: i64) -> Result<u64, String> {
        sqlx::query("DELETE FROM metric_samples WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(|e| e.to_string())
    }
}

fn row_to_sample(row: &sqlx::sqlite::SqliteRow) -> Result<MetricSample, String> {
    Ok(MetricSample {
        metric: row.try_get("metric").map_err(|e| e.to_string())?,
        timestamp: row.try_get("timestamp").map_err(|e| e.to_string())?,
        value: row.try_get("value").map_err(|e| e.to_string())?,
        labels: serde_json::from_str::<BTreeMap<String, String>>(
            row.try_get::<String, _>("labels_json")
                .map_err(|e| e.to_string())?
                .as_str(),
        )
        .map_err(|e| e.to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stores_queries_aggregates_and_cleans_samples() {
        let directory = tempfile::tempdir().unwrap();
        let repository = MetricRepository::open(&directory.path().join("test.sqlite3"))
            .await
            .unwrap();
        let samples = vec![
            MetricSample {
                metric: "cpu.total.usage".into(),
                timestamp: 1_000,
                value: 20.0,
                labels: BTreeMap::new(),
            },
            MetricSample {
                metric: "cpu.total.usage".into(),
                timestamp: 2_000,
                value: 40.0,
                labels: BTreeMap::new(),
            },
        ];
        repository.insert(&samples).await.unwrap();
        let raw = repository
            .raw(&["cpu.total.usage".into()], 0, 3_000, 100)
            .await
            .unwrap();
        assert_eq!(raw.len(), 2);
        let aggregate = repository
            .aggregate(&["cpu.total.usage".into()], 0, 3_000, "avg", None)
            .await
            .unwrap();
        assert_eq!(aggregate.len(), 1);
        assert_eq!(aggregate[0].value, 30.0);
        assert_eq!(repository.cleanup(1_500).await.unwrap(), 1);
    }
}
