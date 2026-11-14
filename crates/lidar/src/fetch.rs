//! IO: download-once disk cache for the public USGS files (no key). Root is `LIDAR_CACHE_DIR`, else
//! `VEXCEL_CACHE_DIR`, else `cache/` (gitignored). Files land under `<root>/lidar/`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Error;

pub fn default_root() -> PathBuf {
    std::env::var_os("LIDAR_CACHE_DIR")
        .or_else(|| std::env::var_os("VEXCEL_CACHE_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cache"))
}

pub fn http() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder()
        .user_agent("golden-hackathon/0.1")
        .timeout(Duration::from_secs(120))
        .build()?)
}

/// The file at `path` if we have it, else GET `url` (retrying 429/5xx with backoff) and save it there.
pub async fn cached(http: &reqwest::Client, url: &str, path: &Path) -> Result<Vec<u8>, Error> {
    if let Ok(b) = tokio::fs::read(path).await {
        return Ok(b);
    }
    let bytes = get(http, url).await?;
    write(path, &bytes).await?;
    Ok(bytes)
}

async fn get(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, Error> {
    let mut wait = Duration::from_millis(500);
    for attempt in 0..5 {
        let res = http.get(url).send().await;
        match res {
            Ok(r) if r.status().is_success() => return Ok(r.bytes().await?.to_vec()),
            Ok(r) if (r.status().as_u16() == 429 || r.status().is_server_error()) && attempt < 4 => {}
            Ok(r) => return Err(Error::Api { status: r.status().as_u16(), url: url.to_string() }),
            Err(e) if attempt < 4 && (e.is_timeout() || e.is_connect()) => {}
            Err(e) => return Err(e.into()),
        }
        tokio::time::sleep(wait).await;
        wait *= 2;
    }
    unreachable!("the last attempt always returns")
}

/// Write via a temp file and rename, so a crash never leaves half a file that looks cached.
async fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}
