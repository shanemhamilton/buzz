//! Shared HTTP retry and in-flight read coordination for the ACP relay client.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::watch;

use super::{RelayError, RestClient};

/// Base retry delays for transient HTTP failures: 500ms, 1s, 2s.
/// Jitter (±20%) is applied at call time via `jittered_duration`.
const REST_RETRY_BASE_DELAYS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
];
/// Do not spin on a malformed or zero-valued rate-limit hint.
const REST_RETRY_MIN_DELAY: Duration = Duration::from_millis(100);
const REST_RATE_LIMIT_MIN_DELAY: Duration = Duration::from_secs(1);
/// Keep a bad or untrusted relay hint from holding an ACP request forever.
/// The retry count remains bounded, so this is a maximum per retry rather than
/// an unbounded wait or an invitation to keep retrying indefinitely.
const REST_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);
const REST_COOLDOWN_CAP: usize = 4096;
const REST_QUERY_FLIGHT_CAP: usize = 256;
/// Error bodies are only inspected for a retry hint. Bound the amount retained
/// while parsing a relay response so an error cannot make the client buffer an
/// unbounded body.
const REST_ERROR_BODY_LIMIT: usize = 16 * 1024;

fn is_retriable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

type RestCooldowns = HashMap<String, tokio::time::Instant>;

/// Cooldowns intentionally live outside `RestClient`: that type has public
/// struct literals throughout the ACP crate, so adding an interior field would
/// be a breaking change. The identity key keeps independent relays and agents
/// isolated while sharing a gate across clones and independently-created
/// clients for the same relay identity.
static REST_COOLDOWNS: OnceLock<Mutex<RestCooldowns>> = OnceLock::new();

#[derive(Clone, Debug)]
enum QueryFlightResult {
    Complete(Result<Value, String>),
    LeaderCancelled,
}

struct QueryFlight {
    result_tx: watch::Sender<Option<QueryFlightResult>>,
}

/// A leader owns one in-flight identical `/query`. Dropping it removes the
/// flight and wakes followers so cancellation cannot strand them forever.
struct QueryFlightGuard {
    key: String,
    flight: Arc<QueryFlight>,
    completed: bool,
}

static REST_QUERY_FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<QueryFlight>>>> = OnceLock::new();

fn rest_cooldowns() -> &'static Mutex<RestCooldowns> {
    REST_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rest_query_flights() -> &'static Mutex<HashMap<String, Arc<QueryFlight>>> {
    REST_QUERY_FLIGHTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_unpoisoned<T>(mutex: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn execute_query(client: &RestClient, body_bytes: &[u8]) -> Result<Value, RelayError> {
    match client.bridge_post("/query", body_bytes).await {
        Ok(response) => response
            .json()
            .await
            .map_err(|error| RelayError::Http(error.to_string())),
        Err(error) => Err(error),
    }
}

impl QueryFlightGuard {
    fn finish(mut self, result: Result<Value, RelayError>) {
        self.completed = true;
        {
            let mut flights = lock_unpoisoned(rest_query_flights());
            if flights
                .get(&self.key)
                .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
            {
                flights.remove(&self.key);
            }
        }
        let result = result.map_err(|error| error.to_string());
        self.flight
            .result_tx
            .send_replace(Some(QueryFlightResult::Complete(result)));
    }
}

impl Drop for QueryFlightGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        {
            let mut flights = lock_unpoisoned(rest_query_flights());
            if flights
                .get(&self.key)
                .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
            {
                flights.remove(&self.key);
            }
        }
        self.flight
            .result_tx
            .send_replace(Some(QueryFlightResult::LeaderCancelled));
    }
}

fn bounded_retry_delay(delay: Duration) -> Duration {
    delay.max(REST_RETRY_MIN_DELAY).min(REST_RETRY_MAX_DELAY)
}

fn bounded_rate_limit_delay(delay: Duration) -> Duration {
    delay
        .max(REST_RATE_LIMIT_MIN_DELAY)
        .min(REST_RETRY_MAX_DELAY)
}

fn retry_after_header_delay(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let now = chrono::Utc::now();
    let milliseconds = date
        .with_timezone(&chrono::Utc)
        .signed_duration_since(now)
        .num_milliseconds();
    if milliseconds <= 0 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_secs(
        (milliseconds as u64).saturating_add(999) / 1000,
    ))
}

fn retry_after_body_delay(body: &str) -> Option<Duration> {
    let parsed = serde_json::from_str::<Value>(body).ok();
    if let Some(object) = parsed.as_ref().and_then(Value::as_object) {
        for key in ["reset_in_secs", "retry_after"] {
            if let Some(seconds) = object.get(key).and_then(Value::as_u64) {
                return Some(Duration::from_secs(seconds));
            }
        }
        if let Some(error) = object.get("error").and_then(Value::as_str) {
            if let Some(seconds) = super::parse_rate_limit_retry_secs(error) {
                return Some(Duration::from_secs(seconds));
            }
        }
    }

    super::parse_rate_limit_retry_secs(parsed.as_ref().and_then(Value::as_str).unwrap_or(body))
        .map(Duration::from_secs)
}

async fn bounded_response_body(response: reqwest::Response) -> Option<String> {
    if response
        .content_length()
        .is_some_and(|length| length > REST_ERROR_BODY_LIMIT as u64)
    {
        return None;
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if bytes.len().saturating_add(chunk.len()) > REST_ERROR_BODY_LIMIT {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).ok()
}

impl RestClient {
    fn identity_key(&self) -> String {
        format!("{}|{}", self.base_url, self.keys.public_key().to_hex())
    }

    fn query_identity_key(&self) -> String {
        use sha2::{Digest, Sha256};

        let auth_tag_hash = hex::encode(Sha256::digest(
            self.auth_tag_json.as_deref().unwrap_or_default().as_bytes(),
        ));
        format!("{}|{auth_tag_hash}", self.identity_key())
    }

    fn active_cooldown(&self) -> Option<tokio::time::Instant> {
        let now = tokio::time::Instant::now();
        let key = self.identity_key();
        let mut cooldowns = lock_unpoisoned(rest_cooldowns());
        cooldowns.retain(|_, deadline| *deadline > now);
        cooldowns.get(&key).copied()
    }

    fn arm_cooldown(&self, delay: Duration) -> bool {
        let Some(deadline) = tokio::time::Instant::now().checked_add(delay) else {
            return false;
        };
        let key = self.identity_key();
        let mut cooldowns = lock_unpoisoned(rest_cooldowns());
        let now = tokio::time::Instant::now();
        cooldowns.retain(|_, existing| *existing > now);
        if !cooldowns.contains_key(&key) && cooldowns.len() >= REST_COOLDOWN_CAP {
            tracing::warn!(
                "HTTP retry cooldown registry is full; refusing to evict active cooldown"
            );
            return false;
        }
        cooldowns
            .entry(key)
            .and_modify(|existing| {
                if *existing < deadline {
                    *existing = deadline;
                }
            })
            .or_insert(deadline);
        true
    }

    async fn wait_for_cooldown(&self) -> Result<(), RelayError> {
        loop {
            let Some(deadline) = self.active_cooldown() else {
                return Ok(());
            };
            let now = tokio::time::Instant::now();
            if deadline <= now {
                return Ok(());
            }
            if deadline.duration_since(now) > REST_RETRY_MAX_DELAY {
                return Err(RelayError::Http(
                    "relay cooldown exceeds the client wait budget".into(),
                ));
            }
            tokio::time::sleep_until(deadline).await;
        }
    }

    fn query_flight_key(&self, body_bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};

        format!(
            "{}|query|{}",
            self.query_identity_key(),
            hex::encode(Sha256::digest(body_bytes))
        )
    }

    pub(super) async fn query_json(&self, body_bytes: Vec<u8>) -> Result<Value, RelayError> {
        let key = self.query_flight_key(&body_bytes);
        loop {
            let registration = {
                let mut flights = lock_unpoisoned(rest_query_flights());
                if let Some(flight) = flights.get(&key) {
                    Some((flight.clone(), false))
                } else if flights.len() >= REST_QUERY_FLIGHT_CAP {
                    None
                } else {
                    let (result_tx, _result_rx) = watch::channel(None);
                    let flight = Arc::new(QueryFlight { result_tx });
                    flights.insert(key.clone(), flight.clone());
                    Some((flight, true))
                }
            };
            let Some((flight, leader)) = registration else {
                return execute_query(self, &body_bytes).await;
            };

            if leader {
                let guard = QueryFlightGuard {
                    key: key.clone(),
                    flight,
                    completed: false,
                };
                let result = execute_query(self, &body_bytes).await;
                let flight_result = result
                    .as_ref()
                    .map(Value::clone)
                    .map_err(|error| RelayError::Http(error.to_string()));
                guard.finish(flight_result);
                return result;
            }

            let mut result_rx = flight.result_tx.subscribe();
            loop {
                if let Some(result) = result_rx.borrow().clone() {
                    match result {
                        QueryFlightResult::Complete(result) => {
                            return result.map_err(RelayError::Http);
                        }
                        QueryFlightResult::LeaderCancelled => break,
                    }
                }
                if result_rx.changed().await.is_err() {
                    break;
                }
            }
            // The leader was cancelled. Re-enter the registry and become a
            // leader if no replacement flight has claimed this key yet.
        }
    }

    pub(super) async fn request_with_retry<F, Fut>(
        &self,
        method: &str,
        path: &str,
        build_request: F,
    ) -> Result<reqwest::Response, RelayError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let mut last_err = None;

        for attempt in 0..=REST_RETRY_BASE_DELAYS.len() {
            // A 429 observed by any concurrent client arms this shared gate.
            // Waiting immediately before building the request prevents a
            // second call from adding pressure during the advertised window.
            self.wait_for_cooldown().await?;
            match build_request().await {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) if is_retriable_status(resp.status()) => {
                    let status = resp.status();
                    let fallback = super::jittered_duration(
                        REST_RETRY_BASE_DELAYS[attempt.min(REST_RETRY_BASE_DELAYS.len() - 1)],
                    );
                    let retry_delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        let header_delay = resp
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|value| value.to_str().ok())
                            .and_then(retry_after_header_delay);
                        let body_delay = if header_delay.is_none() {
                            bounded_response_body(resp)
                                .await
                                .and_then(|body| retry_after_body_delay(&body))
                        } else {
                            None
                        };
                        let server_hint = header_delay.or(body_delay);
                        if let Some(server_hint) = server_hint {
                            if server_hint > REST_RETRY_MAX_DELAY {
                                self.arm_cooldown(server_hint);
                                return Err(RelayError::Http(format!(
                                    "{method} {path} returned HTTP {status} with a retry delay over the client maximum"
                                )));
                            }
                        }
                        bounded_rate_limit_delay(server_hint.unwrap_or(REST_RETRY_MAX_DELAY))
                    } else {
                        drop(resp);
                        bounded_retry_delay(fallback)
                    };
                    if !self.arm_cooldown(retry_delay) {
                        tokio::time::sleep(retry_delay).await;
                    }
                    tracing::warn!(
                        "{method} {path} returned retriable HTTP {status}; retrying after {:.1}s",
                        retry_delay.as_secs_f64()
                    );
                    last_err = Some(RelayError::Http(format!(
                        "{method} {path} returned HTTP {status}"
                    )));
                }
                Ok(resp) => {
                    return Err(RelayError::Http(format!(
                        "{method} {} returned HTTP {}",
                        path,
                        resp.status()
                    )));
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    tracing::warn!("{method} {path} network error: {e}");
                    let fallback = super::jittered_duration(
                        REST_RETRY_BASE_DELAYS[attempt.min(REST_RETRY_BASE_DELAYS.len() - 1)],
                    );
                    if !self.arm_cooldown(fallback) {
                        tokio::time::sleep(fallback).await;
                    }
                    last_err = Some(RelayError::Http(e.to_string()));
                }
                Err(e) => return Err(RelayError::Http(e.to_string())),
            }
        }

        Err(last_err
            .unwrap_or_else(|| RelayError::Http(format!("{method} {path} failed after retries"))))
    }
}

#[cfg(test)]
pub(crate) fn retry_after_header_delay_for_test(value: &str) -> Option<Duration> {
    retry_after_header_delay(value)
}
