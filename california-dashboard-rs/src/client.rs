//! Bounded, rate-limited async HTTP client with auditable outcomes.

use crate::METHOD_VERSION;
use crate::model::{FetchOutcome, FetchSpec, FetchStatus, Provenance, SummaryCard};
use futures::{StreamExt, stream};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderValue, REFERER, RETRY_AFTER};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::{Month, OffsetDateTime};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{Instant, sleep, sleep_until};

pub const DEFAULT_CONCURRENCY: usize = 50;
pub const DEFAULT_MAX_REQUESTS_PER_SECOND: f64 = 1_000.0;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ATTEMPTS: u8 = 4;
const RETRY_BACKOFFS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1_000),
];

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub concurrency: usize,
    pub max_requests_per_second: f64,
    pub timeout: Duration,
    pub max_payload_bytes: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CONCURRENCY,
            max_requests_per_second: DEFAULT_MAX_REQUESTS_PER_SECOND,
            timeout: DEFAULT_TIMEOUT,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
        }
    }
}

impl ClientConfig {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.concurrency == 0 {
            return Err(ClientError::InvalidConfiguration(
                "concurrency must be greater than zero".to_owned(),
            ));
        }
        if !self.max_requests_per_second.is_finite() || self.max_requests_per_second <= 0.0 {
            return Err(ClientError::InvalidConfiguration(
                "max_requests_per_second must be finite and greater than zero".to_owned(),
            ));
        }
        if self.timeout.is_zero() {
            return Err(ClientError::InvalidConfiguration(
                "timeout must be greater than zero".to_owned(),
            ));
        }
        if self.max_payload_bytes == 0 {
            return Err(ClientError::InvalidConfiguration(
                "max_payload_bytes must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct DashboardClient {
    http: reqwest::Client,
    config: ClientConfig,
    rate_limiter: Arc<RateLimiter>,
    concurrency_limiter: Arc<Semaphore>,
}

impl DashboardClient {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/120.0.0.0 Safari/537.36",
            )
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(300))
            .build()
            .map_err(ClientError::BuildHttpClient)?;
        let rate_limiter = Arc::new(RateLimiter::new(config.max_requests_per_second));
        let concurrency_limiter = Arc::new(Semaphore::new(config.concurrency));
        Ok(Self {
            http,
            config,
            rate_limiter,
            concurrency_limiter,
        })
    }

    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Fetches one specification and always returns its original metadata.
    pub async fn fetch(&self, spec: &FetchSpec) -> FetchOutcome {
        self.fetch_owned(spec.clone()).await
    }

    /// Fetches concurrently while returning results in exact input order.
    pub async fn fetch_all(&self, specs: &[FetchSpec]) -> Vec<FetchOutcome> {
        let mut indexed: Vec<(usize, FetchOutcome)> =
            stream::iter(specs.iter().cloned().enumerate().map(|(index, spec)| {
                let client = self.clone();
                async move { (index, client.fetch_owned(spec).await) }
            }))
            .buffer_unordered(self.config.concurrency)
            .collect()
            .await;
        indexed.sort_by_key(|(index, _)| *index);
        indexed.into_iter().map(|(_, outcome)| outcome).collect()
    }

    async fn fetch_owned(&self, spec: FetchSpec) -> FetchOutcome {
        let _permit = self
            .concurrency_limiter
            .acquire()
            .await
            .expect("client concurrency semaphore is never closed");
        for attempt in 1..=MAX_ATTEMPTS {
            self.rate_limiter.acquire().await;
            let response = self
                .http
                .get(&spec.url)
                .header(REFERER, "https://www.caschooldashboard.org/")
                .header(ACCEPT, "application/json, text/plain, */*")
                .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
                .send()
                .await;

            let mut response = match response {
                Ok(response) => response,
                Err(error) => {
                    if attempt < MAX_ATTEMPTS {
                        sleep(retry_backoff(attempt)).await;
                        continue;
                    }
                    let provenance = provenance(&spec, attempt, None, None);
                    return failure(
                        spec,
                        provenance,
                        FetchStatus::TransportError {
                            message: error.to_string(),
                        },
                    );
                }
            };

            let status = response.status();
            let status_code = status.as_u16();
            let declared_bytes = response.content_length();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(parse_retry_after);

            if retryable_status(status_code) && attempt < MAX_ATTEMPTS {
                let delay = retry_after
                    .map(|server_delay| server_delay.max(retry_backoff(attempt)))
                    .unwrap_or_else(|| retry_backoff(attempt));
                sleep(delay).await;
                continue;
            }

            if declared_bytes.is_some_and(|length| length > self.config.max_payload_bytes as u64) {
                let provenance = provenance(&spec, attempt, Some(status_code), None);
                return failure(
                    spec,
                    provenance,
                    FetchStatus::PayloadTooLarge {
                        limit_bytes: self.config.max_payload_bytes,
                        declared_bytes,
                    },
                );
            }

            let payload = match read_bounded(&mut response, self.config.max_payload_bytes).await {
                Ok(payload) => payload,
                Err(ReadBodyError::TooLarge) => {
                    let provenance = provenance(&spec, attempt, Some(status_code), None);
                    return failure(
                        spec,
                        provenance,
                        FetchStatus::PayloadTooLarge {
                            limit_bytes: self.config.max_payload_bytes,
                            declared_bytes,
                        },
                    );
                }
                Err(ReadBodyError::Transport(error)) => {
                    if attempt < MAX_ATTEMPTS {
                        sleep(retry_backoff(attempt)).await;
                        continue;
                    }
                    let provenance = provenance(&spec, attempt, Some(status_code), None);
                    return failure(
                        spec,
                        provenance,
                        FetchStatus::TransportError {
                            message: error.to_string(),
                        },
                    );
                }
            };
            let provenance = provenance(&spec, attempt, Some(status_code), Some(&payload));

            if !status.is_success() {
                return failure(spec, provenance, FetchStatus::HttpError { status_code });
            }
            if payload.is_empty() {
                return failure(spec, provenance, FetchStatus::EmptyPayload);
            }

            return match SummaryCard::from_payload(&spec, &payload, provenance.clone()) {
                Ok(card) => FetchOutcome {
                    spec,
                    status: FetchStatus::Success,
                    card: Some(card),
                    provenance,
                },
                Err(error) => failure(
                    spec,
                    provenance,
                    FetchStatus::InvalidJson {
                        message: error.to_string(),
                    },
                ),
            };
        }

        unreachable!("fetch loop always returns on its final attempt")
    }
}

fn failure(spec: FetchSpec, provenance: Provenance, status: FetchStatus) -> FetchOutcome {
    FetchOutcome {
        spec,
        status,
        card: None,
        provenance,
    }
}

fn retryable_status(status_code: u16) -> bool {
    matches!(status_code, 408 | 429) || (500..=599).contains(&status_code)
}

fn retry_backoff(attempt: u8) -> Duration {
    RETRY_BACKOFFS[usize::from(attempt.saturating_sub(1)).min(RETRY_BACKOFFS.len() - 1)]
}

async fn read_bounded(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ReadBodyError> {
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(limit);
    let mut body = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response.chunk().await.map_err(ReadBodyError::Transport)? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(ReadBodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

enum ReadBodyError {
    TooLarge,
    Transport(reqwest::Error),
}

fn provenance(
    spec: &FetchSpec,
    attempts: u8,
    http_status: Option<u16>,
    payload: Option<&[u8]>,
) -> Provenance {
    Provenance {
        source_url: spec.url.clone(),
        retrieved_at_utc: retrieval_timestamp(),
        payload_sha256: payload.map(sha256_hex),
        payload_bytes: payload.map_or(0, <[u8]>::len),
        http_status,
        attempts,
        method_version: METHOD_VERSION.to_owned(),
    }
}

fn retrieval_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    now.format(&Rfc3339)
        .unwrap_or_else(|_| now.unix_timestamp().to_string())
}

fn sha256_hex(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn parse_retry_after(value: &HeaderValue) -> Option<Duration> {
    let value = value.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let target = parse_http_date(value)?;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    (target > now).then(|| Duration::from_secs((target - now) as u64))
}

/// Parses the IMF-fixdate form required for HTTP-date, without locale state.
fn parse_http_date(value: &str) -> Option<i64> {
    let fields: Vec<_> = value.split_ascii_whitespace().collect();
    if fields.len() != 6 || !fields[0].ends_with(',') || fields[5] != "GMT" {
        return None;
    }
    let day = fields[1].parse::<u8>().ok()?;
    let month = match fields[2] {
        "Jan" => Month::January,
        "Feb" => Month::February,
        "Mar" => Month::March,
        "Apr" => Month::April,
        "May" => Month::May,
        "Jun" => Month::June,
        "Jul" => Month::July,
        "Aug" => Month::August,
        "Sep" => Month::September,
        "Oct" => Month::October,
        "Nov" => Month::November,
        "Dec" => Month::December,
        _ => return None,
    };
    let year = fields[3].parse::<i32>().ok()?;
    let mut clock = fields[4].split(':');
    let hour = clock.next()?.parse::<u8>().ok()?;
    let minute = clock.next()?.parse::<u8>().ok()?;
    let second = clock.next()?.parse::<u8>().ok()?;
    if clock.next().is_some() {
        return None;
    }
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let time = time::Time::from_hms(hour, minute, second).ok()?;
    Some(date.with_time(time).assume_utc().unix_timestamp())
}

struct RateLimiter {
    interval: Duration,
    next_request: Mutex<Instant>,
}

impl RateLimiter {
    fn new(requests_per_second: f64) -> Self {
        let interval =
            Duration::from_secs_f64(1.0 / requests_per_second).max(Duration::from_nanos(1));
        Self {
            interval,
            next_request: Mutex::new(Instant::now()),
        }
    }

    async fn acquire(&self) {
        let target = {
            let mut next_request = self.next_request.lock().await;
            let now = Instant::now();
            if *next_request < now {
                *next_request = now;
            }
            let target = *next_request;
            *next_request += self.interval;
            target
        };
        sleep_until(target).await;
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid client configuration: {0}")]
    InvalidConfiguration(String),
    #[error("failed to construct HTTP client: {0}")]
    BuildHttpClient(#[source] reqwest::Error),
}

#[cfg(test)]
mod tests {
    use super::{parse_http_date, retry_backoff};
    use std::time::Duration;

    #[test]
    fn backoff_schedule_is_exact() {
        assert_eq!(retry_backoff(1), Duration::from_millis(250));
        assert_eq!(retry_backoff(2), Duration::from_millis(500));
        assert_eq!(retry_backoff(3), Duration::from_millis(1_000));
    }

    #[test]
    fn parses_standard_http_date() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(parse_http_date("not a date"), None);
    }
}
