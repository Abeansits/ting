//! Axum server for the live dashboard. Binds loopback only — the dashboard
//! is an opt-in personal dev UI, never a public service.

use anyhow::{Context, Result};
use async_stream::stream;
use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{
        Html, IntoResponse, Json,
        sse::{Event as SseEvent, Sse},
    },
    routing::get,
};
use notify::{
    Config as NotifyConfig, Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde_json::Value;
use std::convert::Infallible;
use std::fs::File;
use std::future::Future;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::Stream;

use crate::dashboard_state;
use crate::events::{DashboardEvent, EVENT_LOG_FILENAME, EventType, event_log_path};

const DASHBOARD_HTML: &str = include_str!("static/dashboard.html");
const DASHBOARD_CSS: &str = include_str!("static/dashboard.css");
const DASHBOARD_JS: &str = include_str!("static/dashboard.js");

/// Room for a burst of events between the tailer and the slowest subscriber.
/// 1024 covers a full forum's worth of events with plenty of headroom.
const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Cadence of the `event: ping` heartbeat the SSE endpoint emits between
/// real events so proxies and `EventSource` clients don't time out.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct AppState {
    forum_dir: PathBuf,
    events_tx: broadcast::Sender<DashboardEvent>,
}

/// Must be called inside a Tokio runtime — spawns the JSONL tailer task.
pub(crate) fn router(forum_dir: PathBuf) -> Router {
    let (events_tx, _) = broadcast::channel::<DashboardEvent>(EVENT_CHANNEL_CAPACITY);
    spawn_tailer(forum_dir.clone(), events_tx.clone());
    Router::new()
        .route("/", get(serve_dashboard))
        .route("/static/dashboard.css", get(serve_css))
        .route("/static/dashboard.js", get(serve_js))
        .route("/api/state", get(serve_state))
        .route("/api/events", get(serve_events))
        .with_state(AppState {
            forum_dir,
            events_tx,
        })
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

async fn serve_css() -> impl IntoResponse {
    static_asset("text/css; charset=utf-8", DASHBOARD_CSS)
}

async fn serve_js() -> impl IntoResponse {
    static_asset("text/javascript; charset=utf-8", DASHBOARD_JS)
}

fn static_asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        body,
    )
}

async fn serve_state(State(app): State<AppState>) -> ApiResult<Json<Value>> {
    match dashboard_state::read_state(&app.forum_dir).map_err(internal_error)? {
        Some(state) => Ok(Json(serde_json::to_value(&state).map_err(internal_error)?)),
        None => Err(not_found("dashboard state not yet available")),
    }
}

async fn serve_events(
    State(app): State<AppState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe before reading the file so any event appended during backlog
    // read is captured by the broadcast and deduped by seq.
    let mut rx = app.events_tx.subscribe();
    let forum_dir = app.forum_dir.clone();

    let sse_stream = stream! {
        if let Ok(Some(state)) = dashboard_state::read_state(&forum_dir)
            && let Ok(body) = serde_json::to_string(&state)
        {
            yield Ok(SseEvent::default().event("init").data(body));
        }

        let (backlog, mut max_seq) = match read_full_log(&forum_dir) {
            Ok(events) => {
                let last = events.last().map(|e| e.seq).unwrap_or(0);
                (events, last)
            }
            Err(e) => {
                eprintln!("sse backlog read error: {e:#}");
                (Vec::new(), 0)
            }
        };

        let mut completed = false;
        for ev in backlog {
            if matches!(ev.event_type, EventType::ForumComplete) {
                completed = true;
            }
            yield Ok(event_to_sse(&ev));
            if completed {
                return;
            }
        }

        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // discard the immediate first tick

        loop {
            tokio::select! {
                recv = rx.recv() => match recv {
                    Ok(ev) => {
                        if ev.seq <= max_seq {
                            continue; // already delivered via backlog
                        }
                        max_seq = ev.seq;
                        let done = matches!(ev.event_type, EventType::ForumComplete);
                        yield Ok(event_to_sse(&ev));
                        if done {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("sse client lagged, closing stream ({n} events dropped)");
                        return;
                    }
                },
                _ = ticker.tick() => {
                    yield Ok(SseEvent::default().event("ping"));
                }
            }
        }
    };

    Sse::new(sse_stream)
}

fn event_to_sse(ev: &DashboardEvent) -> SseEvent {
    // DashboardEvent is owned data with a derived Serialize; to_string cannot fail.
    let body = serde_json::to_string(ev).expect("DashboardEvent serialize");
    SseEvent::default()
        .event("update")
        .id(ev.seq.to_string())
        .data(body)
}

fn read_full_log(forum_dir: &Path) -> Result<Vec<DashboardEvent>> {
    let path = event_log_path(forum_dir);
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("Failed to open {}", path.display())),
    };
    let mut events = Vec::new();
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.with_context(|| format!("Failed to read {}:{}", path.display(), line_no + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<DashboardEvent>(&line) {
            Ok(ev) => events.push(ev),
            Err(e) => warn_malformed(&path, Some(line_no + 1), &e),
        }
    }
    Ok(events)
}

fn warn_malformed(path: &Path, line_no: Option<usize>, err: &dyn std::fmt::Display) {
    match line_no {
        Some(n) => eprintln!(
            "warning: skipping malformed event at {}:{}: {}",
            path.display(),
            n,
            err
        ),
        None => eprintln!(
            "warning: skipping malformed event at {}: {}",
            path.display(),
            err
        ),
    }
}

/// If the tailer task errors out, clients still get backlog via file reads
/// on connect but lose live streaming.
fn spawn_tailer(forum_dir: PathBuf, events_tx: broadcast::Sender<DashboardEvent>) {
    tokio::spawn(async move {
        if let Err(e) = run_tailer(&forum_dir, &events_tx).await {
            eprintln!("event tailer error: {e:#}");
        }
    });
}

async fn run_tailer(forum_dir: &Path, events_tx: &broadcast::Sender<DashboardEvent>) -> Result<()> {
    let events_path = event_log_path(forum_dir);
    let target_name = std::ffi::OsStr::new(EVENT_LOG_FILENAME);

    // Unbounded because notify's callback is sync and must not block.
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<NotifyEvent>| {
            let Ok(event) = res else { return };
            if event
                .paths
                .iter()
                .any(|p| p.file_name() == Some(target_name))
            {
                let _ = notify_tx.send(());
            }
        },
        NotifyConfig::default(),
    )
    .with_context(|| "Failed to create event-log watcher")?;
    watcher
        .watch(forum_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("Failed to watch {}", forum_dir.display()))?;

    // Cursor starts at 0 + initial catch-up scan: covers the window between
    // the watcher registering and any early event append from a forum task
    // racing this setup. Clients dedup any overlap via seq.
    let mut cursor = catch_up(&events_path, 0, events_tx);

    while notify_rx.recv().await.is_some() {
        // Coalesce bursts of notifications — one read per quiet period.
        while notify_rx.try_recv().is_ok() {}
        cursor = catch_up(&events_path, cursor, events_tx);
    }
    Ok(())
}

fn catch_up(events_path: &Path, cursor: u64, events_tx: &broadcast::Sender<DashboardEvent>) -> u64 {
    match tail_since(events_path, cursor) {
        Ok((new_cursor, new_events)) => {
            for ev in new_events {
                // Err just means no subscribers; benign.
                let _ = events_tx.send(ev);
            }
            new_cursor
        }
        Err(e) => {
            eprintln!("event tailer read error: {e:#}");
            cursor
        }
    }
}

/// Read any complete lines from `path` starting at byte offset `cursor`.
/// On truncation (file shorter than cursor) restarts reading from 0.
fn tail_since(path: &Path, cursor: u64) -> Result<(u64, Vec<DashboardEvent>)> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((cursor, Vec::new())),
        Err(e) => return Err(e).with_context(|| format!("Failed to open {}", path.display())),
    };
    let len = file.metadata()?.len();
    let start = if len < cursor { 0 } else { cursor };
    file.seek(SeekFrom::Start(start))?;

    let mut reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut new_cursor = start;
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            break;
        }
        if !buf.ends_with('\n') {
            // Partial line — leave cursor at its start; next wake will re-read.
            break;
        }
        new_cursor += n as u64;
        let line = buf.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<DashboardEvent>(line) {
            Ok(ev) => events.push(ev),
            Err(e) => warn_malformed(path, None, &e),
        }
    }
    Ok((new_cursor, events))
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
    use chrono::Utc;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    use tokio_stream::StreamExt as _;
    use tower::ServiceExt;

    use crate::dashboard_state::{DashboardState, ForumStatus, write_state};
    use crate::events::{EVENT_VERSION, append_event};

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

    fn make_event(seq: u64, event_type: EventType) -> DashboardEvent {
        DashboardEvent {
            version: EVENT_VERSION,
            seq,
            forum_id: "ting-test-0001".into(),
            timestamp: Utc::now(),
            event_type,
            payload: json!({ "round": seq }),
        }
    }

    fn content_type(resp: &axum::response::Response) -> String {
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn get_root_returns_html_shell() {
        let dir = tmp_dir("root");
        let resp = router(dir)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = content_type(&resp);
        assert!(ct.starts_with("text/html"), "content-type was {ct}");
        let body = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(body.contains("Ting Dashboard"));
        assert!(body.contains("/api/state"));
        for id in [
            "forum-id",
            "topic",
            "rounds-grid",
            "metrics",
            "gauge-fill",
            "synthesis-info",
            "connection",
        ] {
            assert!(body.contains(id), "missing section id `{id}` in HTML shell");
        }
        assert!(body.contains("/static/dashboard.css"));
        assert!(body.contains("/static/dashboard.js"));
    }

    #[tokio::test]
    async fn static_css_served_with_css_content_type() {
        let dir = tmp_dir("css");
        let resp = router(dir)
            .oneshot(
                Request::get("/static/dashboard.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = content_type(&resp);
        assert!(ct.starts_with("text/css"), "content-type was {ct}");
        assert_eq!(
            resp.headers()
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
        );
        let body = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(body.contains("--bg"), "expected CSS vars in body");
    }

    #[tokio::test]
    async fn static_js_served_with_js_content_type() {
        let dir = tmp_dir("js");
        let resp = router(dir)
            .oneshot(
                Request::get("/static/dashboard.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = content_type(&resp);
        assert!(
            ct.starts_with("text/javascript") || ct.starts_with("application/javascript"),
            "content-type was {ct}"
        );
        assert_eq!(
            resp.headers()
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
        );
        let body = String::from_utf8(body_bytes(resp).await).unwrap();
        // Every event type the server may broadcast must have a JS handler.
        for needle in [
            "EventSource",
            "forum_started",
            "round_started",
            "participant_response",
            "synthesis",
            "classifier_metrics",
            "metric_scores",
            "convergence",
            "forum_complete",
        ] {
            assert!(body.contains(needle), "JS missing handler for `{needle}`");
        }
        // Closes the stream when init reports an already-completed forum so
        // clients don't sit in "live" waiting for an update that will never come.
        assert!(
            body.contains("\"completed\"") && body.contains("es.close()"),
            "JS must close SSE when init snapshot reports completed state",
        );
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

    #[test]
    fn tail_since_reads_complete_lines_and_holds_partial() {
        let dir = tmp_dir("tail-partial");
        let path = event_log_path(&dir);
        let mut body = serde_json::to_string(&make_event(1, EventType::RoundStarted)).unwrap();
        body.push('\n');
        body.push_str(&serde_json::to_string(&make_event(2, EventType::RoundStarted)).unwrap());
        body.push('\n');
        // Partial third line, no trailing newline.
        body.push_str("{\"version\":1,\"seq\":3"); // incomplete JSON, no \n
        fs::write(&path, &body).unwrap();

        let (cursor, events) = tail_since(&path, 0).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
        // Cursor should sit at the start of the partial line.
        let full = fs::read(&path).unwrap();
        assert!(cursor < full.len() as u64);
        assert_eq!(&full[cursor as usize..], b"{\"version\":1,\"seq\":3");
    }

    #[test]
    fn tail_since_reads_only_new_bytes_since_cursor() {
        let dir = tmp_dir("tail-incremental");
        let path = event_log_path(&dir);
        append_event(&dir, &make_event(1, EventType::RoundStarted)).unwrap();
        let (cursor_a, events_a) = tail_since(&path, 0).unwrap();
        assert_eq!(events_a.len(), 1);

        append_event(&dir, &make_event(2, EventType::RoundStarted)).unwrap();
        append_event(&dir, &make_event(3, EventType::RoundStarted)).unwrap();
        let (_cursor_b, events_b) = tail_since(&path, cursor_a).unwrap();
        assert_eq!(events_b.len(), 2);
        assert_eq!(events_b[0].seq, 2);
        assert_eq!(events_b[1].seq, 3);
    }

    #[test]
    fn tail_since_skips_malformed_lines() {
        let dir = tmp_dir("tail-malformed");
        let path = event_log_path(&dir);
        append_event(&dir, &make_event(1, EventType::RoundStarted)).unwrap();
        use std::fs::OpenOptions;
        use std::io::Write;
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"not-json\n")
            .unwrap();
        append_event(&dir, &make_event(2, EventType::RoundStarted)).unwrap();

        let (_cursor, events) = tail_since(&path, 0).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
    }

    /// Regression: tailer must broadcast events that were already in the
    /// file at startup. Without the initial catch-up scan, a forum task
    /// racing the tailer's watcher setup silently loses its first events
    /// for any client already subscribed.
    #[tokio::test]
    async fn tailer_catches_up_preexisting_events_on_startup() {
        let dir = tmp_dir("tailer-catchup");
        for seq in 1..=3u64 {
            append_event(&dir, &make_event(seq, EventType::RoundStarted)).unwrap();
        }
        let (tx, mut rx) = broadcast::channel::<DashboardEvent>(32);
        spawn_tailer(dir.clone(), tx);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut got = Vec::new();
        while got.len() < 3 && Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(ev)) => got.push(ev),
                _ => continue,
            }
        }
        assert_eq!(got.len(), 3, "expected 3 events, got {}", got.len());
        assert_eq!(got.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn tailer_broadcasts_appended_events() {
        let dir = tmp_dir("tailer-broadcast");
        let (tx, mut rx) = broadcast::channel::<DashboardEvent>(32);
        spawn_tailer(dir.clone(), tx);

        // Give notify a moment to register the watch on the forum dir.
        tokio::time::sleep(Duration::from_millis(50)).await;

        for seq in 1..=3u64 {
            append_event(&dir, &make_event(seq, EventType::RoundStarted)).unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut got = Vec::new();
        while got.len() < 3 && Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(ev)) => got.push(ev),
                _ => continue,
            }
        }
        assert_eq!(got.len(), 3, "expected 3 events, got {}", got.len());
        assert!(
            got.iter()
                .enumerate()
                .all(|(i, ev)| ev.seq == (i + 1) as u64),
            "seq not monotonic: {:?}",
            got.iter().map(|e| e.seq).collect::<Vec<_>>()
        );
    }

    /// Read the SSE body as a string, stopping after `min_bytes` have arrived
    /// or `deadline` expires. Never blocks forever.
    async fn collect_sse(body: axum::body::Body, min_bytes: usize, deadline: Duration) -> String {
        let mut data_stream = body.into_data_stream();
        let mut out = Vec::new();
        let end = Instant::now() + deadline;
        while out.len() < min_bytes && Instant::now() < end {
            let remaining = end.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, data_stream.next()).await {
                Ok(Some(Ok(bytes))) => out.extend_from_slice(&bytes),
                _ => break,
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn sse_update_seqs(body: &str) -> Vec<u64> {
        let mut seqs = Vec::new();
        let mut is_update = false;
        for line in body.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                is_update = rest.trim() == "update";
            } else if is_update && let Some(rest) = line.strip_prefix("data:") {
                if let Ok(ev) = serde_json::from_str::<DashboardEvent>(rest.trim()) {
                    seqs.push(ev.seq);
                }
                is_update = false;
            } else if line.is_empty() {
                is_update = false;
            }
        }
        seqs
    }

    #[tokio::test]
    async fn api_events_replays_backlog_then_streams_live() {
        let dir = tmp_dir("api-events-live");
        append_event(&dir, &make_event(1, EventType::ForumStarted)).unwrap();
        append_event(&dir, &make_event(2, EventType::RoundStarted)).unwrap();

        let app = router(dir.clone());
        let resp = app
            .oneshot(Request::get("/api/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(ct.starts_with("text/event-stream"), "content-type was {ct}");

        let body = resp.into_body();
        // Give the tailer a beat to register, then push a live event.
        tokio::time::sleep(Duration::from_millis(100)).await;
        append_event(&dir, &make_event(3, EventType::RoundStarted)).unwrap();
        append_event(&dir, &make_event(4, EventType::ForumComplete)).unwrap();

        // Collect enough bytes to likely include all 4 update events.
        let text = collect_sse(body, 4_000, Duration::from_secs(3)).await;
        let seqs = sse_update_seqs(&text);
        assert!(
            seqs.len() >= 4,
            "expected at least 4 update events, got {seqs:?}\nbody:\n{text}"
        );
        assert_eq!(&seqs[..4], &[1, 2, 3, 4], "seqs: {seqs:?}\nbody:\n{text}");
    }

    #[tokio::test]
    async fn api_events_replays_completed_forum_and_ends() {
        let dir = tmp_dir("api-events-completed");
        append_event(&dir, &make_event(1, EventType::ForumStarted)).unwrap();
        append_event(&dir, &make_event(2, EventType::ForumComplete)).unwrap();

        let app = router(dir);
        let resp = app
            .oneshot(Request::get("/api/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Stream should end quickly after the final event.
        let text = collect_sse(resp.into_body(), 10_000, Duration::from_secs(2)).await;
        let seqs = sse_update_seqs(&text);
        assert_eq!(seqs, vec![1, 2], "body:\n{text}");
    }

    #[tokio::test]
    async fn api_events_emits_init_when_state_present() {
        let dir = tmp_dir("api-events-init");
        let mut state =
            DashboardState::new("ting-2026-04-19-abcd1234", "topic", vec!["codex".into()], 2);
        state.latest_seq = 0;
        write_state(&dir, &state).unwrap();
        append_event(&dir, &make_event(1, EventType::ForumStarted)).unwrap();
        append_event(&dir, &make_event(2, EventType::ForumComplete)).unwrap();

        let app = router(dir);
        let resp = app
            .oneshot(Request::get("/api/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let text = collect_sse(resp.into_body(), 4_000, Duration::from_secs(2)).await;
        assert!(
            text.contains("event: init") || text.contains("event:init"),
            "body:\n{text}"
        );
        assert!(text.contains("ting-2026-04-19-abcd1234"), "body:\n{text}");
    }
}
