//! HTTP to Vexcel's DSM extract, with backoff. The key goes in the `X-API-KEY` header only,
//! never in a URL or an error message.

use std::time::Duration;

use crate::auth::ApiKey;
use crate::tiles::{tile_wkt, TileId};
use crate::Error;

pub const DSM_URL: &str = "https://api.vexcelgroup.com/v2/dsm/extract";
const MAX_ATTEMPTS: u32 = 6;

pub fn http() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(180)).build()?)
}

/// Fetch one tile's GeoTIFF. Retries 429 / 5xx / network errors with backoff.
pub async fn fetch_tile(http: &reqwest::Client, key: &ApiKey, t: TileId) -> Result<Vec<u8>, Error> {
    let wkt = tile_wkt(t);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = http
            .get(DSM_URL)
            .query(&[("wkt", wkt.as_str()), ("srid", "4326")])
            .header("X-API-KEY", key.expose())
            .send()
            .await;
        let retry_after = match resp {
            Ok(r) if r.status().is_success() => return Ok(r.bytes().await?.to_vec()),
            Ok(r) if is_retryable(r.status().as_u16()) => header_secs(&r),
            Ok(r) => {
                let status = r.status().as_u16();
                let body: String = r.text().await.unwrap_or_default().chars().take(300).collect();
                return Err(Error::Api { status, body });
            }
            Err(e) if attempt < MAX_ATTEMPTS && (e.is_timeout() || e.is_connect()) => None,
            Err(e) => return Err(e.into()),
        };
        if attempt >= MAX_ATTEMPTS {
            return Err(Error::Api { status: 429, body: format!("gave up after {attempt} attempts") });
        }
        tokio::time::sleep(backoff_delay(attempt, retry_after)).await;
    }
}

fn header_secs(r: &reqwest::Response) -> Option<u64> {
    r.headers().get("retry-after")?.to_str().ok()?.trim().parse().ok()
}

pub fn is_retryable(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Honour Retry-After if the server sent one, else 1, 2, 4, 8, 16 s. Never more than 60 s.
pub fn backoff_delay(attempt: u32, retry_after_s: Option<u64>) -> Duration {
    let s = retry_after_s.unwrap_or(1u64 << (attempt.saturating_sub(1)).min(4));
    Duration::from_secs(s.min(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        let secs: Vec<u64> = (1..=7).map(|a| backoff_delay(a, None).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 16, 16]);
        assert_eq!(backoff_delay(1, Some(30)).as_secs(), 30);
        assert_eq!(backoff_delay(1, Some(9999)).as_secs(), 60);
    }

    #[test]
    fn which_statuses_retry() {
        assert!(is_retryable(429) && is_retryable(503));
        assert!(!is_retryable(400) && !is_retryable(401) && !is_retryable(404));
    }
}
