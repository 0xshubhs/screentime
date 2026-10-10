//! Local dashboard: the embedded TypeScript UI plus a small JSON API.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use rust_embed::RustEmbed;
use screentime_core::report::{self, Filter};
use serde::Deserialize;

use crate::tracker::WebState;
use crate::{now_ms, Shared};

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct Assets;

/// Longest range one report may cover.
const MAX_DAYS: i64 = 92;

pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/api/report", get(report_handler))
        .route("/api/status", get(status))
        .route("/api/icon", get(icon))
        .route("/api/web", post(web))
        .route("/api/pause", post(pause))
        .fallback(get(asset))
        .layer(middleware::from_fn(local_only))
        .with_state(shared)
}

/// Rejects requests whose Host is not loopback, which blocks DNS rebinding from web pages.
async fn local_only(req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
    if matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "screentime only answers on localhost").into_response()
    }
}

#[derive(Deserialize)]
struct ReportQuery {
    from: NaiveDate,
    to: Option<NaiveDate>,
    app: Option<String>,
    domain: Option<String>,
    category: Option<String>,
}

async fn report_handler(State(s): State<Arc<Shared>>, Query(q): Query<ReportQuery>) -> Response {
    let to = q.to.unwrap_or(q.from);
    if to < q.from || (to - q.from).num_days() >= MAX_DAYS {
        return (StatusCode::BAD_REQUEST, "bad date range").into_response();
    }
    let filter = match (q.app, q.domain, q.category) {
        (Some(a), _, _) => Filter::App(a),
        (_, Some(d), _) => Filter::Domain(d),
        (_, _, Some(c)) => Filter::Category(c),
        _ => Filter::None,
    };
    let apps = s.apps.read().unwrap();
    match s.with(|db, _, rules| report::build(db, rules, &apps, q.from, to, &filter, now_ms())) {
        Some(r) => Json(r).into_response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn status(State(s): State<Arc<Shared>>) -> Response {
    let (focus, domain, idle, locked, paused) = s
        .with(|_, t, _| Ok((t.focus.clone(), t.current_domain().map(str::to_string), t.idle, t.locked, t.paused)))
        .unwrap_or_default();
    let web_ms = s.web_seen_ms.load(Ordering::Relaxed);
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "tracking": s.tracking,
        "extension": s.extension_ok.load(Ordering::Relaxed),
        "browser_extension": web_ms > 0 && now_ms() - web_ms < 10 * 60_000,
        "focus": focus.map(|f| s.apps.read().unwrap().info(&f).name),
        "domain": domain,
        "idle": idle,
        "locked": locked,
        "paused": paused,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct IconQuery {
    id: String,
}

async fn icon(State(s): State<Arc<Shared>>, Query(q): Query<IconQuery>) -> Response {
    let Some(path) = s.apps.read().unwrap().icon_path(&q.id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.to_string()), (header::CACHE_CONTROL, "max-age=86400".into())], bytes)
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct WebReport {
    browser: String,
    domain: Option<String>,
    #[serde(default)]
    audible: bool,
}

/// Active tab reports from the browser extension. The custom header cannot be sent by a
/// plain cross-origin page without a CORS preflight, which we never approve.
async fn web(State(s): State<Arc<Shared>>, headers: HeaderMap, Json(w): Json<WebReport>) -> StatusCode {
    if !s.tracking || headers.get("x-screentime").is_none() {
        return StatusCode::FORBIDDEN;
    }
    let domain = w
        .domain
        .map(|d| d.trim().trim_start_matches("www.").to_ascii_lowercase())
        .filter(|d| !d.is_empty() && d.len() < 256);
    s.web_seen_ms.store(now_ms(), Ordering::Relaxed);
    let state = WebState { domain, audible: w.audible };
    s.with(|db, t, _| t.on_web(db, now_ms(), &w.browser, state));
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct PauseReq {
    paused: bool,
}

async fn pause(State(s): State<Arc<Shared>>, headers: HeaderMap, Json(p): Json<PauseReq>) -> StatusCode {
    if !s.tracking || headers.get("x-screentime").is_none() {
        return StatusCode::FORBIDDEN;
    }
    s.with(|db, t, _| t.on_pause(db, now_ms(), p.paused));
    StatusCode::NO_CONTENT
}

async fn asset(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let file = Assets::get(path).or_else(|| Assets::get("index.html"));
    match file {
        Some(f) => {
            let mime = f.metadata.mimetype().to_string();
            ([(header::CONTENT_TYPE, mime)], Body::from(f.data.into_owned())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            "UI not built: run `npm --prefix ui ci && npm --prefix ui run build`, then rebuild screentimed",
        )
            .into_response(),
    }
}
