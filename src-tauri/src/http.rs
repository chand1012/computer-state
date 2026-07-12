use crate::{
    model::{is_known_metric, MetricSample},
    state::AppState,
};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use rocket::{
    catch,
    fairing::{Fairing, Info, Kind},
    get,
    http::Status,
    request::{FromRequest, Outcome},
    response::{content::RawText, status::Custom},
    serde::json::Json,
    Build, Data, Request, Response, Rocket, State,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct RequestId(String);
struct RequestTracing;

#[rocket::async_trait]
impl Fairing for RequestTracing {
    fn info(&self) -> Info {
        Info {
            name: "request correlation",
            kind: Kind::Request | Kind::Response,
        }
    }

    async fn on_request(&self, request: &mut Request<'_>, _data: &mut Data<'_>) {
        request.local_cache(|| {
            RequestId(format!(
                "{:x}-{:x}",
                chrono::Utc::now().timestamp_millis(),
                REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
        });
    }

    async fn on_response<'r>(&self, request: &'r Request<'_>, response: &mut Response<'r>) {
        let request_id = &request.local_cache(|| RequestId("unknown".into())).0;
        response.set_raw_header("X-Request-ID", request_id);
        tracing::debug!(request_id, method = %request.method(), uri = %request.uri(), status = response.status().code, "HTTP request completed");
    }
}

pub struct AllowedPeer;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AllowedPeer {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let Some(state) = request.rocket().state::<Arc<AppState>>() else {
            return Outcome::Error((Status::ServiceUnavailable, ()));
        };
        let allowed = state.settings.get().await.http.allowed_interfaces;
        let peer = request.client_ip();
        if peer.is_some_and(|ip| peer_allowed(ip, &allowed)) {
            Outcome::Success(AllowedPeer)
        } else {
            Outcome::Error((Status::Forbidden, ()))
        }
    }
}

fn peer_allowed(ip: IpAddr, allowed: &[String]) -> bool {
    allowed.iter().any(|entry| match entry.as_str() {
        "loopback" => ip.is_loopback(),
        "tailscale" => is_tailscale(ip),
        literal => literal
            .parse::<IpAddr>()
            .is_ok_and(|candidate| candidate == ip),
    })
}

fn is_tailscale(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    }
}

#[get("/metrics")]
async fn metrics(
    _peer: AllowedPeer,
    state: &State<Arc<AppState>>,
) -> Result<RawText<String>, Status> {
    let samples = state
        .collect(false)
        .await
        .map_err(|_| Status::ServiceUnavailable)?;
    Ok(RawText(prometheus_text(&samples)))
}

#[derive(Serialize)]
struct LatestResponse {
    timestamp: Option<String>,
    metrics: Vec<MetricSample>,
}

#[get("/latest")]
async fn latest(_peer: AllowedPeer, state: &State<Arc<AppState>>) -> Json<LatestResponse> {
    let settings = state.settings.get().await;
    let samples = state
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
        .collect::<Vec<_>>();
    let timestamp = samples
        .iter()
        .map(|sample| sample.timestamp)
        .max()
        .and_then(ms_to_rfc3339);
    Json(LatestResponse {
        timestamp,
        metrics: samples,
    })
}

type ApiResult = Result<Json<Value>, Custom<Json<Value>>>;

#[allow(clippy::too_many_arguments)]
#[get("/query?<metric>&<from>&<to>&<date>&<range>&<aggregation>&<interval>")]
async fn query(
    _peer: AllowedPeer,
    state: &State<Arc<AppState>>,
    metric: Vec<String>,
    from: Option<&str>,
    to: Option<&str>,
    date: Option<&str>,
    range: Option<&str>,
    aggregation: Option<&str>,
    interval: Option<&str>,
) -> ApiResult {
    let mut metric = metric;
    metric.sort();
    metric.dedup();
    if metric.is_empty() || metric.iter().any(|name| !is_known_metric(name)) {
        return bad_request("one or more valid metric parameters are required");
    }
    if metric.len() > 16 {
        return bad_request("a query may request at most 16 metrics");
    }
    let mode_count =
        usize::from(from.is_some()) + usize::from(date.is_some()) + usize::from(range.is_some());
    if mode_count != 1 {
        return bad_request("use exactly one of from, date, or range");
    }
    if interval.is_some() && aggregation.is_none() {
        return bad_request("interval requires aggregation");
    }
    let operation = aggregation.unwrap_or("");
    if !operation.is_empty() && !["avg", "min", "max", "sum", "count"].contains(&operation) {
        return bad_request("unsupported aggregation");
    }

    let now = Utc::now().timestamp_millis();
    let end = match to {
        Some(value) => parse_timestamp(value)?,
        None => now,
    };
    let start = if let Some(value) = from {
        parse_timestamp(value)?
    } else if let Some(value) = date {
        let day = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| api_error(Status::BadRequest, "invalid date"))?;
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .ok_or_else(|| api_error(Status::BadRequest, "ambiguous local date"))?
            .with_timezone(&Utc)
            .timestamp_millis()
    } else {
        end - parse_duration(range.unwrap())
            .map_err(|message| api_error(Status::BadRequest, message))?
    };
    let effective_end = if let Some(value) = date {
        let day = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .unwrap()
            .succ_opt()
            .unwrap();
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .ok_or_else(|| api_error(Status::BadRequest, "ambiguous local date"))?
            .with_timezone(&Utc)
            .timestamp_millis()
    } else {
        end
    };
    if start >= effective_end {
        return bad_request("query start must precede query end");
    }
    let retention = state.settings.get().await.collection.retention_days as i64 * 86_400_000;
    let bounded_start = start.max(now - retention);

    let samples = if operation.is_empty() {
        state
            .repository
            .raw(
                &metric,
                bounded_start,
                effective_end,
                50_000 / metric.len() as i64,
            )
            .await
    } else {
        let bucket = interval
            .map(parse_duration)
            .transpose()
            .map_err(|message| api_error(Status::BadRequest, message))?;
        state
            .repository
            .aggregate(&metric, bounded_start, effective_end, operation, bucket)
            .await
    }
    .map_err(|_| api_error(Status::InternalServerError, "query failed"))?;

    Ok(Json(json!({
        "from": ms_to_rfc3339(bounded_start),
        "to": ms_to_rfc3339(effective_end),
        "aggregation": aggregation,
        "interval": interval,
        "samples": samples
    })))
}

pub fn build(state: Arc<AppState>, port: u16) -> Rocket<Build> {
    let config = rocket::Config::figment()
        .merge(("address", "0.0.0.0"))
        .merge(("port", port))
        .merge(("log_level", "critical"));
    rocket::custom(config)
        .attach(RequestTracing)
        .manage(state)
        .mount("/", rocket::routes![metrics, latest, query])
        .register(
            "/",
            rocket::catchers![
                bad_request_catcher,
                forbidden_catcher,
                not_found_catcher,
                internal_catcher,
                unavailable_catcher
            ],
        )
}

#[catch(400)]
fn bad_request_catcher() -> Json<Value> {
    Json(json!({ "error": "bad request" }))
}
#[catch(403)]
fn forbidden_catcher() -> Json<Value> {
    Json(json!({ "error": "request rejected by network policy" }))
}
#[catch(404)]
fn not_found_catcher() -> Json<Value> {
    Json(json!({ "error": "endpoint not found" }))
}
#[catch(500)]
fn internal_catcher() -> Json<Value> {
    Json(json!({ "error": "internal server error" }))
}
#[catch(503)]
fn unavailable_catcher() -> Json<Value> {
    Json(json!({ "error": "service unavailable" }))
}

fn parse_timestamp(value: &str) -> Result<i64, Custom<Json<Value>>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc).timestamp_millis())
        .map_err(|_| api_error(Status::BadRequest, "invalid RFC 3339 timestamp"))
}

fn parse_duration(value: &str) -> Result<i64, &'static str> {
    if value.len() < 2 {
        return Err("invalid duration");
    }
    let (number, unit) = value.split_at(value.len() - 1);
    let amount = number.parse::<i64>().map_err(|_| "invalid duration")?;
    if amount <= 0 {
        return Err("duration must be positive");
    }
    let multiplier = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return Err("duration unit must be m, h, d, or w"),
    };
    amount
        .checked_mul(multiplier)
        .ok_or("duration is too large")
}

fn prometheus_text(samples: &[MetricSample]) -> String {
    let mut output = String::new();
    for sample in samples {
        let name = format!("computer_state_{}", sample.metric.replace('.', "_"));
        output.push_str(&name);
        if !sample.labels.is_empty() {
            output.push('{');
            for (index, (key, value)) in sample.labels.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&format!(
                    "{}=\"{}\"",
                    key.replace('.', "_"),
                    value.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
            output.push('}');
        }
        output.push_str(&format!(" {}\n", sample.value));
    }
    output
}

fn ms_to_rfc3339(value: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(value).map(|value| value.to_rfc3339())
}

fn bad_request<T>(message: &'static str) -> Result<T, Custom<Json<Value>>> {
    Err(api_error(Status::BadRequest, message))
}

fn api_error(status: Status, message: impl Into<String>) -> Custom<Json<Value>> {
    Custom(status, Json(json!({ "error": message.into() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("6h"), Ok(21_600_000));
        assert!(parse_duration("0m").is_err());
    }

    #[test]
    fn recognizes_tailscale_addresses() {
        assert!(is_tailscale("100.100.1.2".parse().unwrap()));
        assert!(!is_tailscale("192.168.1.2".parse().unwrap()));
    }
}
