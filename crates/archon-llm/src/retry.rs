use std::time::Duration;

use anyhow::Result;
use reqwest::Response;

/// Configuration for HTTP request retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 30_000,
        }
    }
}

/// Whether an HTTP status code is retryable.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 529)
}

/// Compute the delay for a given retry attempt.
/// Prefers the `Retry-After` header if present, otherwise uses exponential backoff.
fn compute_delay(response: &Response, attempt: u32, config: &RetryConfig) -> Duration {
    // Check Retry-After header first
    if let Some(retry_after) = response.headers().get("retry-after") {
        if let Ok(secs) = retry_after.to_str().unwrap_or("").parse::<f64>() {
            let ms = (secs * 1000.0) as u64;
            return Duration::from_millis(ms.min(config.max_delay_ms));
        }
    }

    // Exponential backoff: base_delay * 2^attempt, capped at max_delay
    let delay_ms = config.base_delay_ms * 2u64.pow(attempt);
    Duration::from_millis(delay_ms.min(config.max_delay_ms))
}

/// Execute an HTTP request with retry logic.
///
/// `make_request` is called on each attempt and must return a `reqwest::Response`.
/// Only retries on transient status codes (429, 500, 502, 503, 529) and network errors.
/// Client errors (400, 401, 404, etc.) fail immediately.
pub async fn with_retry<F, Fut>(config: &RetryConfig, make_request: F) -> Result<Response>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Response>>,
{
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=config.max_retries {
        match make_request().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if response.status().is_success() || response.status().is_redirection() {
                    return Ok(response);
                }

                if !is_retryable_status(status) {
                    // Non-retryable error — fail immediately
                    let body = response.text().await.unwrap_or_default();
                    anyhow::bail!("HTTP {status}: {body}");
                }

                if attempt < config.max_retries {
                    let delay = compute_delay(&response, attempt, config);
                    eprintln!(
                        "[retry] HTTP {status}, attempt {}/{}, waiting {:.1}s",
                        attempt + 1,
                        config.max_retries,
                        delay.as_secs_f64()
                    );
                    tokio::time::sleep(delay).await;
                } else {
                    let body = response.text().await.unwrap_or_default();
                    anyhow::bail!(
                        "HTTP {status} after {} retries: {body}",
                        config.max_retries
                    );
                }
            }
            Err(e) => {
                if attempt < config.max_retries {
                    let delay_ms = config.base_delay_ms * 2u64.pow(attempt);
                    let delay =
                        Duration::from_millis(delay_ms.min(config.max_delay_ms));
                    eprintln!(
                        "[retry] Network error: {e}, attempt {}/{}, waiting {:.1}s",
                        attempt + 1,
                        config.max_retries,
                        delay.as_secs_f64()
                    );
                    tokio::time::sleep(delay).await;
                    last_error = Some(e);
                } else {
                    return Err(e.context(format!(
                        "Failed after {} retries",
                        config.max_retries
                    )));
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Retry loop exhausted")))
}
