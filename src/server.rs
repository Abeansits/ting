//! Axum server for the live dashboard. Binds loopback only — the dashboard
//! is an opt-in personal dev UI, never a public service.

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, Json},
    routing::get,
};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use tokio::net::TcpListener;

use crate::dashboard_state;

const DASHBOARD_HTML: &str = include_str!("static/dashboard.html");

#[derive(Clone)]
struct AppState {
    forum_dir: PathBuf,
}

/// Build the router for a forum directory. Exposed to tests so they can
/// exercise handlers via `tower::ServiceExt::oneshot` without binding a port.
pub(crate) fn router(forum_dir: PathBuf) -> Router {
    Router::new()
        .route("/", get(serve_dashboard))
        .route("/api/state", get(serve_state))
        .with_state(AppState { forum_dir })
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

async fn serve_state(State(app): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    match dashboard_state::read_state(&app.forum_dir).map_err(internal_error)? {
        Some(state) => Ok(Json(serde_json::to_value(&state).map_err(internal_error)?)),
        None => Err(not_found("dashboard state not yet available")),
    }
}

type ApiError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T> = std::result::Result<T, ApiError>;

fn error_body(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "error": msg.into() })))
}

fn not_found(msg: impl Into<String>) -> ApiError {
    error_body(StatusCode::NOT_FOUND, msg)
}

fn internal_error<E: std::fmt::Display>(e: E) -> ApiError {
    error_body(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Bind a loopback `TcpListener` for the dashboard. Split from `serve` so the
/// caller can verify the bind before committing to long-running work
/// (e.g. spawning the blocking forum task) — avoids orphaning that work if the
/// server fails to start.
pub async fn bind_loopback(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind {addr}"))
}

/// Serve the dashboard on an already-bound listener until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    forum_dir: PathBuf,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    axum::serve(listener, router(forum_dir))
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum server error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tower::ServiceExt;

    use crate::dashboard_state::{DashboardState, ForumStatus, write_state};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ting-server-{label}-{pid}-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
        to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn get_root_returns_html_shell() {
        let dir = tmp_dir("root");
        let resp = router(dir)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(ct.starts_with("text/html"), "content-type was {ct}");
        let body = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(body.contains("Ting Dashboard"));
        assert!(body.contains("/api/state"));
    }

    #[tokio::test]
    async fn get_api_state_returns_404_when_missing() {
        let dir = tmp_dir("state-missing");
        let resp = router(dir)
            .oneshot(Request::get("/api/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert!(body.get("error").is_some(), "expected error field: {body}");
    }

    #[tokio::test]
    async fn get_api_state_returns_snapshot_when_present() {
        let dir = tmp_dir("state-present");
        let mut state = DashboardState::new(
            "ting-2026-04-19-abcd1234",
            "Is the sky falling?",
            vec!["codex".into(), "gemini".into()],
            3,
        );
        state.status = ForumStatus::InProgress;
        state.latest_seq = 5;
        write_state(&dir, &state).unwrap();

        let resp = router(dir)
            .oneshot(Request::get("/api/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(body["forum_id"], "ting-2026-04-19-abcd1234");
        assert_eq!(body["status"], "in_progress");
        assert_eq!(body["latest_seq"], 5);
        assert_eq!(body["participants"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn serve_shuts_down_on_signal() {
        let dir = tmp_dir("shutdown");
        let listener = bind_loopback(0).await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(serve(listener, dir, async move {
            let _ = rx.await;
        }));

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        tx.send(()).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("server did not shut down in time")
            .expect("server task panicked")
            .unwrap();
    }
}
