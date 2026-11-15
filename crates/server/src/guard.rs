//! Guardrails for running golden on the public internet (burvy.dev). All of it is set by environment variables,
//! read once at startup; with none set the server behaves as before, apart from the rate limits and the Claude
//! daily cap, whose defaults are generous for one person and tight for a crowd.
//!
//! - `GOLDEN_OFFLINE=1`: cache only. No Vexcel (heights, photos, tiles, mesh, detection), no USGS LiDAR, no Google
//!   geocoding: `vexcel::auth::ApiKey::from_env` reports no key, `lidar` refuses to download, and `secrets::get`
//!   hides the Google key. What is on disk is served; anything else is a 503 saying the demo is cache only.
//! - `GOLDEN_CLAUDE=off`: never call Claude (the agent and the chat run on the offline reader).
//! - `GOLDEN_CLAUDE_DAILY_MAX` (default 300): Claude calls per UTC day, for the whole server. Past it the agent and
//!   the chat fall back to the offline reader until midnight UTC.
//! - Per-IP token buckets (`limit`), 429 + `Retry-After` when empty:
//!   - AI (`/api/chat`, `/api/agent`): `GOLDEN_RATE_AI_PER_MIN` (default 6, burst 2x);
//!   - heavy (search, horizon, scene, photo, objects): `GOLDEN_RATE_HEAVY_PER_MIN` (default 30, burst 2x);
//!   - every other `/api/*` call: `GOLDEN_RATE_PER_MIN` (default 300, burst 2x).
//!   Map tiles, mesh, scene binaries, photo JPEGs and the web app are not limited (cache reads, many per view).
//! - `GOLDEN_HEAVY_CONCURRENCY` (default 4): AI and heavy requests running at once; the rest wait up to 30 s, then 503.
//! - Client IP: when the TCP peer is loopback (nginx on the same machine) or listed in `GOLDEN_TRUSTED_PROXIES`
//!   (comma list of IPs), it's `CF-Connecting-IP`, else the first `X-Forwarded-For` hop; otherwise the peer address,
//!   so a visitor connecting directly can't pick their own bucket with a header.
//! - `GOLDEN_CORS_ORIGINS` (default none: burvy.dev serves the page and the API on one origin): comma list of
//!   origins allowed to call the API from a page served elsewhere.
//! - `GOLDEN_WARM=0`: don't load the preset demo grids at startup.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{NaiveDate, Utc};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::state::{ApiErr, AppState};

/// Scene binaries kept in memory; older ones are still served from their disk copy.
pub const MAX_SCENES: usize = 32;
/// Buckets kept per class before idle (full) ones are dropped.
const MAX_BUCKETS: usize = 10_000;
/// How long an AI or heavy request waits for a free slot before giving up.
const QUEUE_WAIT: Duration = Duration::from_secs(30);

/// The guard settings and their live state (buckets, the Claude budget, the heavy-request slots).
pub struct Guard {
    pub offline: bool,
    pub claude_on: bool,
    pub claude_daily_max: u32,
    /// Proxies (besides loopback) whose forwarded-for headers are believed.
    pub trusted: Vec<IpAddr>,
    pub cors: Vec<String>,
    pub warm: bool,
    ai: Limiter,
    heavy: Limiter,
    light: Limiter,
    slots: Arc<tokio::sync::Semaphore>,
    budget: Mutex<Budget>,
    start: Instant,
}

/// The settings, from the environment (once; later calls return the same guard).
pub fn get() -> &'static Guard {
    static GUARD: OnceLock<Guard> = OnceLock::new();
    GUARD.get_or_init(|| Guard::from_vars(|k| std::env::var(k).ok()))
}

/// Pure: is an env value "on"? (`1`, `true`, `yes`, `on`, any case.)
pub fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("1" | "true" | "yes" | "on"))
}

impl Guard {
    /// Settings from `var` (the environment in the server, a map in tests).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Guard {
        let num = |k: &str, d: f64| var(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|n| *n >= 0.0).unwrap_or(d);
        let off = |k: &str| matches!(var(k).map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("0" | "false" | "no" | "off"));
        let cors = var("GOLDEN_CORS_ORIGINS").unwrap_or_default();
        Guard {
            offline: truthy(var("GOLDEN_OFFLINE").as_deref()),
            claude_on: !off("GOLDEN_CLAUDE"),
            claude_daily_max: num("GOLDEN_CLAUDE_DAILY_MAX", 300.0) as u32,
            trusted: var("GOLDEN_TRUSTED_PROXIES").unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect(),
            cors: origins(&cors),
            warm: !off("GOLDEN_WARM"),
            ai: Limiter::per_min(num("GOLDEN_RATE_AI_PER_MIN", 6.0)),
            heavy: Limiter::per_min(num("GOLDEN_RATE_HEAVY_PER_MIN", 30.0)),
            light: Limiter::per_min(num("GOLDEN_RATE_PER_MIN", 300.0)),
            slots: Arc::new(tokio::sync::Semaphore::new(num("GOLDEN_HEAVY_CONCURRENCY", 4.0).max(1.0) as usize)),
            budget: Mutex::new(Budget::default()),
            start: Instant::now(),
        }
    }

    /// One line for the startup log (no secrets in it).
    pub fn summary(&self) -> String {
        format!(
            "guard: {}; Claude {}; per IP per minute: AI {}, heavy {}, other {}; CORS {}{}",
            if self.offline { "OFFLINE (cache only, no downloads)" } else { "online (downloads allowed)" },
            if self.claude_on { format!("on, at most {} calls a day", self.claude_daily_max) } else { "off".into() },
            self.ai.per_min,
            self.heavy.per_min,
            self.light.per_min,
            if self.cors.is_empty() { "off".into() } else { self.cors.join(" ") },
            if self.trusted.is_empty() { String::new() } else { format!("; trusted proxies {:?}", self.trusted) },
        )
    }

    /// May the server call Claude at all right now (on, and today's budget not spent)? Doesn't spend.
    pub fn claude_allowed(&self) -> bool {
        self.claude_on && self.budget.lock().unwrap().left(today(), self.claude_daily_max) > 0
    }

    /// Spend one Claude call from today's budget; `Err` (a message for the logs) if Claude is off or it's spent.
    pub fn claude_take(&self) -> Result<(), String> {
        if !self.claude_on {
            return Err("Claude is turned off (GOLDEN_CLAUDE=off)".into());
        }
        if self.budget.lock().unwrap().take(today(), self.claude_daily_max) {
            Ok(())
        } else {
            Err(format!("Claude's daily limit ({} calls, GOLDEN_CLAUDE_DAILY_MAX) is reached until midnight UTC", self.claude_daily_max))
        }
    }

    fn now_s(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// Pure: a comma list of origins, `off`/empty for none.
pub fn origins(list: &str) -> Vec<String> {
    if list.trim().eq_ignore_ascii_case("off") {
        return vec![];
    }
    list.split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect()
}

// ---- Claude daily budget ----

/// Claude calls spent on one UTC day.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Budget {
    day: Option<NaiveDate>,
    used: u32,
}

impl Budget {
    /// Calls left on `day` (a new day starts from zero).
    pub fn left(&self, day: NaiveDate, max: u32) -> u32 {
        if self.day == Some(day) { max.saturating_sub(self.used) } else { max }
    }

    /// Spend one call on `day` if any are left.
    pub fn take(&mut self, day: NaiveDate, max: u32) -> bool {
        if self.day != Some(day) {
            *self = Budget { day: Some(day), used: 0 };
        }
        if self.used < max {
            self.used += 1;
            true
        } else {
            false
        }
    }
}

// ---- per-IP rate limit ----

/// A token bucket: `tokens` left as of `at` (seconds since the guard started).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bucket {
    tokens: f64,
    at: f64,
}

/// Pure: take one token from `b` at `now_s`, refilling `per_min` a minute up to `burst`. `Err(seconds to wait)`
/// when empty. A missing bucket starts full.
pub fn take_token(b: Option<Bucket>, now_s: f64, per_min: f64, burst: f64) -> (Bucket, Result<(), f64>) {
    let rate = per_min / 60.0;
    let tokens = match b {
        Some(b) => (b.tokens + (now_s - b.at).max(0.0) * rate).min(burst),
        None => burst,
    };
    if tokens >= 1.0 {
        (Bucket { tokens: tokens - 1.0, at: now_s }, Ok(()))
    } else {
        let wait = if rate > 0.0 { (1.0 - tokens) / rate } else { 3600.0 };
        (Bucket { tokens, at: now_s }, Err(wait))
    }
}

/// Buckets per client for one class of request.
pub struct Limiter {
    per_min: f64,
    burst: f64,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl Limiter {
    pub fn per_min(per_min: f64) -> Limiter {
        Limiter { per_min, burst: (per_min * 2.0).max(1.0), buckets: Mutex::default() }
    }

    /// One request from `ip` at `now_s`: `Err(seconds to wait)` if over the limit.
    pub fn check(&self, ip: IpAddr, now_s: f64) -> Result<(), f64> {
        let mut map = self.buckets.lock().unwrap();
        if map.len() >= MAX_BUCKETS {
            // drop clients that have been away long enough to be full again
            let (rate, burst) = (self.per_min / 60.0, self.burst);
            map.retain(|_, b| b.tokens + (now_s - b.at) * rate < burst);
            if map.len() >= MAX_BUCKETS {
                map.clear();
            }
        }
        let (b, ok) = take_token(map.get(&ip).copied(), now_s, self.per_min, self.burst);
        map.insert(ip, b);
        ok
    }
}

/// Which limit a request falls under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Calls Claude (and searches): `/api/chat`, `/api/agent`.
    Ai,
    /// Builds a grid, a search or a scene, or may fetch an image.
    Heavy,
    /// Any other `/api/*` call.
    Light,
    /// Not limited: static files, map tiles, mesh, scene binaries and photo JPEGs (cache reads), health.
    Free,
}

/// Pure: the class of a request path.
pub fn class(path: &str) -> Class {
    let path = path.trim_end_matches('/');
    match path {
        "/api/chat" | "/api/agent" => Class::Ai,
        "/api/search" | "/api/horizon" | "/api/scene" | "/api/photo" | "/api/objects" => Class::Heavy,
        "/api/health" => Class::Free,
        p if p.starts_with("/api/tiles/") || p.starts_with("/api/mesh/") => Class::Free,
        p if p.starts_with("/api/scene/") || p.starts_with("/api/photo/") => Class::Free,
        p if p.starts_with("/api/") || p == "/api" => Class::Light,
        _ => Class::Free,
    }
}

/// Pure: the client's IP. When the peer is a proxy we trust (loopback, or in `trusted`), from its headers:
/// Cloudflare's `CF-Connecting-IP`, else the first `X-Forwarded-For` hop. Otherwise, or with no usable header,
/// the peer itself.
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted: &[IpAddr]) -> Option<IpAddr> {
    if peer.is_some_and(|p| p.is_loopback() || trusted.contains(&p)) {
        let h = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let from_headers = h("cf-connecting-ip")
            .or_else(|| h("x-forwarded-for").and_then(|v| v.split(',').next()))
            .and_then(|v| v.trim().parse().ok());
        if from_headers.is_some() {
            return from_headers;
        }
    }
    peer
}

/// Middleware: rate-limit by class and client IP, then cap how many AI/heavy requests run at once.
pub async fn limit(State(g): State<&'static Guard>, req: Request, next: Next) -> Response {
    let class = class(req.uri().path());
    if class == Class::Free || req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    // No address at all only happens in tests (no socket); everyone then shares one bucket.
    let ip = client_ip(req.headers(), peer, &g.trusted).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let limiter = match class {
        Class::Ai => &g.ai,
        Class::Heavy => &g.heavy,
        _ => &g.light,
    };
    if let Err(wait) = limiter.check(ip, g.now_s()) {
        return too_many(wait);
    }
    if class == Class::Light {
        return next.run(req).await;
    }
    match tokio::time::timeout(QUEUE_WAIT, g.slots.clone().acquire_owned()).await {
        Ok(Ok(_slot)) => next.run(req).await,
        _ => ApiErr(StatusCode::SERVICE_UNAVAILABLE, "golden is busy right now; try again in a minute".into()).into_response(),
    }
}

fn too_many(wait_s: f64) -> Response {
    let secs = wait_s.ceil().max(1.0) as u64;
    let mut res = ApiErr(StatusCode::TOO_MANY_REQUESTS, format!("too many requests; try again in {secs} s")).into_response();
    if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
        res.headers_mut().insert(header::RETRY_AFTER, v);
    }
    res
}

// ---- CORS ----

/// CORS for the listed origins (GET/POST with a JSON body); `None` if the list is empty.
pub fn cors(g: &Guard) -> Option<CorsLayer> {
    if g.cors.is_empty() {
        return None;
    }
    let list: Vec<HeaderValue> = g.cors.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(list))
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::CONTENT_TYPE])
            .max_age(Duration::from_secs(3600)),
    )
}

// ---- offline answers ----

/// The 503 for something the cache doesn't hold while offline.
pub fn offline_err(what: &str) -> ApiErr {
    ApiErr(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("{what}: this server only serves cached demo areas (Downtown LA, Princeton) and that isn't cached"),
    )
}

// ---- bounded scene map ----

/// Insert into a map that holds at most `cap` entries, dropping arbitrary old ones first.
pub fn bounded_insert<K: Eq + Hash + Clone, V>(map: &mut HashMap<K, V>, key: K, value: V, cap: usize) {
    if !map.contains_key(&key) {
        while map.len() >= cap.max(1) {
            let Some(k) = map.keys().next().cloned() else { break };
            map.remove(&k);
        }
    }
    map.insert(key, value);
}

// ---- warm the demo grids ----

/// Load the preset demo areas' height grids (from the cache when offline), so the first search after a boot
/// doesn't spend 10 s reading the disk. Runs in the background; failures are logged and skipped.
pub fn warm(st: Arc<AppState>) {
    if !get().warm {
        return;
    }
    tokio::spawn(async move {
        for p in crate::agent::read::PRESETS.iter().take(crate::state::MAX_GRIDS) {
            let t0 = Instant::now();
            let need = golden_core::search::skyline_need_m(p.radius_m);
            match st.grids.around(p.lat, p.lon, need).await {
                Ok(_) => eprintln!("warm: {} ready in {:.1} s", p.name, t0.elapsed().as_secs_f64()),
                Err(e) => eprintln!("warm: {} skipped ({})", p.name, e.1),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(pairs: &[(&str, &str)]) -> Guard {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Guard::from_vars(|k| m.get(k).cloned())
    }

    #[test]
    fn defaults_are_online_with_claude_capped() {
        let g = guard(&[]);
        assert!(!g.offline && g.claude_on && g.warm && g.trusted.is_empty());
        assert_eq!(g.claude_daily_max, 300);
        assert!(g.cors.is_empty(), "same origin behind nginx: no CORS by default");
        assert!(cors(&g).is_none());
    }

    #[test]
    fn env_turns_things_off() {
        let g = guard(&[("GOLDEN_OFFLINE", "1"), ("GOLDEN_CLAUDE", "off"), ("GOLDEN_CORS_ORIGINS", "off"), ("GOLDEN_WARM", "0")]);
        assert!(g.offline && !g.claude_on && !g.warm && g.cors.is_empty());
        assert!(!g.claude_allowed());
        assert!(g.claude_take().is_err());
        let g = guard(&[("GOLDEN_OFFLINE", "no"), ("GOLDEN_CORS_ORIGINS", "https://a.dev/, http://localhost:5173")]);
        assert!(!g.offline);
        assert_eq!(g.cors, ["https://a.dev", "http://localhost:5173"]);
    }

    #[test]
    fn claude_budget_stops_at_the_cap_and_resets_each_day() {
        let (d1, d2) = (NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(), NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
        let mut b = Budget::default();
        assert!(b.take(d1, 2) && b.take(d1, 2));
        assert!(!b.take(d1, 2));
        assert_eq!(b.left(d1, 2), 0);
        assert_eq!(b.left(d2, 2), 2);
        assert!(b.take(d2, 2));
        assert!(!Budget::default().take(d1, 0), "a cap of 0 allows nothing");
        let g = guard(&[("GOLDEN_CLAUDE_DAILY_MAX", "1")]);
        assert!(g.claude_allowed() && g.claude_take().is_ok());
        assert!(!g.claude_allowed() && g.claude_take().is_err());
    }

    #[test]
    fn a_bucket_allows_its_burst_then_refills() {
        let mut b = None;
        for _ in 0..4 {
            let (nb, ok) = take_token(b, 0.0, 2.0, 4.0);
            assert!(ok.is_ok());
            b = Some(nb);
        }
        let (nb, ok) = take_token(b, 0.0, 2.0, 4.0);
        assert!((ok.unwrap_err() - 30.0).abs() < 1e-9, "2 a minute: one more in 30 s");
        let (_, ok) = take_token(Some(nb), 30.0, 2.0, 4.0);
        assert!(ok.is_ok());
        // a long absence refills only up to the burst
        let (nb, _) = take_token(Some(nb), 1e6, 2.0, 4.0);
        assert!((nb.tokens - 3.0).abs() < 1e-9);
    }

    #[test]
    fn limiter_keeps_clients_apart() {
        let l = Limiter::per_min(1.0); // burst 2
        let (a, b): (IpAddr, IpAddr) = ("1.2.3.4".parse().unwrap(), "5.6.7.8".parse().unwrap());
        assert!(l.check(a, 0.0).is_ok() && l.check(a, 0.0).is_ok());
        assert!(l.check(a, 0.0).is_err());
        assert!(l.check(b, 0.0).is_ok());
    }

    #[test]
    fn classes() {
        assert_eq!(class("/api/chat"), Class::Ai);
        assert_eq!(class("/api/agent/"), Class::Ai);
        for p in ["/api/search", "/api/horizon", "/api/scene", "/api/photo", "/api/objects"] {
            assert_eq!(class(p), Class::Heavy, "{p}");
        }
        for p in ["/api/sun", "/api/buildings", "/api/buildings/costar:1", "/api/nope"] {
            assert_eq!(class(p), Class::Light, "{p}");
        }
        for p in ["/", "/index.html", "/api/health", "/api/tiles/vexcel/16/1/2", "/api/mesh/tile", "/api/scene/ab.bin", "/api/photo/x.jpg", "/apix"] {
            assert_eq!(class(p), Class::Free, "{p}");
        }
    }

    #[test]
    fn client_ip_believes_headers_only_from_a_trusted_peer() {
        let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "9.9.9.9, 10.0.0.2".parse().unwrap());
        // nginx on loopback: the first hop is the visitor
        assert_eq!(client_ip(&h, Some(ip("127.0.0.1")), &[]), Some(ip("9.9.9.9")));
        assert_eq!(client_ip(&h, Some(ip("::1")), &[]), Some(ip("9.9.9.9")));
        // a direct visitor can't choose their bucket with a header
        assert_eq!(client_ip(&h, Some(ip("5.5.5.5")), &[]), Some(ip("5.5.5.5")));
        // unless that peer is a listed proxy
        assert_eq!(client_ip(&h, Some(ip("10.1.1.1")), &[ip("10.1.1.1")]), Some(ip("9.9.9.9")));
        h.insert("cf-connecting-ip", "8.8.4.4".parse().unwrap());
        assert_eq!(client_ip(&h, Some(ip("127.0.0.1")), &[]), Some(ip("8.8.4.4")), "Cloudflare's header first");
        let mut junk = HeaderMap::new();
        junk.insert("x-forwarded-for", "not an ip".parse().unwrap());
        assert_eq!(client_ip(&junk, Some(ip("127.0.0.1")), &[]), Some(ip("127.0.0.1")));
        assert_eq!(client_ip(&HeaderMap::new(), None, &[]), None);
    }

    /// A tiny app behind the guard, with `pairs` as its environment.
    fn guarded(pairs: &[(&str, &str)]) -> axum::Router {
        use axum::routing::{get, post};
        let g: &'static Guard = Box::leak(Box::new(guard(pairs)));
        let r = axum::Router::new()
            .route("/api/search", post(|| async { "ok" }))
            .route("/api/tiles/vexcel/1/2/3", get(|| async { "tile" }))
            .layer(axum::middleware::from_fn_with_state(g, limit));
        match cors(g) {
            Some(c) => r.layer(c),
            None => r,
        }
    }

    async fn status_of(app: &axum::Router, req: axum::http::Request<axum::body::Body>) -> (StatusCode, HeaderMap) {
        use tower::ServiceExt;
        let res = app.clone().oneshot(req).await.unwrap();
        (res.status(), res.headers().clone())
    }

    #[tokio::test]
    async fn over_the_limit_is_429_with_retry_after_and_tiles_stay_free() {
        use axum::body::Body;
        use axum::http::Request;
        let app = guarded(&[("GOLDEN_RATE_HEAVY_PER_MIN", "1")]); // burst 2
        let search = || Request::post("/api/search").body(Body::empty()).unwrap();
        assert_eq!(status_of(&app, search()).await.0, StatusCode::OK);
        assert_eq!(status_of(&app, search()).await.0, StatusCode::OK);
        let (status, headers) = status_of(&app, search()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(headers[header::RETRY_AFTER].to_str().unwrap().parse::<u64>().unwrap() >= 1);
        for _ in 0..20 {
            let tile = Request::get("/api/tiles/vexcel/1/2/3").body(Body::empty()).unwrap();
            assert_eq!(status_of(&app, tile).await.0, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn cors_allows_only_the_listed_origins() {
        use axum::body::Body;
        use axum::http::Request;
        let app = guarded(&[("GOLDEN_CORS_ORIGINS", "https://burvy.dev")]);
        let preflight = |origin: &str| {
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/search")
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .unwrap()
        };
        let (_, h) = status_of(&app, preflight("https://burvy.dev")).await;
        assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "https://burvy.dev");
        let (_, h) = status_of(&app, preflight("https://evil.example")).await;
        assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }

    #[test]
    fn bounded_insert_caps_the_map() {
        let mut m = HashMap::new();
        for i in 0..10 {
            bounded_insert(&mut m, i, i, 3);
        }
        assert_eq!(m.len(), 3);
        assert_eq!(m.get(&9), Some(&9));
        bounded_insert(&mut m, 9, 99, 3); // replacing keeps the others
        assert_eq!(m.len(), 3);
    }
}
