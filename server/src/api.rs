//! REST + WebSocket API and embedded frontend.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::at::AtError;
use crate::euicc::ProfileAction;
use crate::events::Event;
use crate::modem::{Modem, NotConnected, RadioMode};

type AppState = Arc<Modem>;

pub struct ApiError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = if self.0.is::<NotConnected>() || matches!(self.0.downcast_ref(), Some(AtError::Closed)) {
            StatusCode::SERVICE_UNAVAILABLE
        } else if self.0.is::<AtError>() {
            StatusCode::BAD_GATEWAY
        } else if self.0.is::<rusqlite::Error>() {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::BAD_REQUEST
        };
        (code, Json(json!({ "error": format!("{:#}", self.0) }))).into_response()
    }
}

type ApiResult<T = Value> = Result<Json<T>, ApiError>;

fn ok<T: serde::Serialize>(v: T) -> ApiResult {
    Ok(Json(serde_json::to_value(v)?))
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/status/refresh", post(refresh))
        .route("/devices", get(devices))
        .route("/sms", get(list_sms).post(send_sms))
        .route("/sms/conversations", get(conversations))
        .route("/sms/sync", post(sync_sms))
        .route("/sms/settings", get(sms_settings).put(set_sms_settings))
        .route("/sms/{id}", delete(delete_sms))
        .route("/sms/peer/{peer}", delete(delete_peer))
        .route("/sms/peer/{peer}/read", post(mark_read))
        .route("/data", post(set_data))
        .route("/data/counters/reset", post(reset_counters))
        .route("/radio", post(set_radio))
        .route("/modem/reboot", post(reboot))
        .route("/sim/pin", post(enter_pin))
        .route("/sim/msisdn", post(set_msisdn))
        .route("/apn", get(get_apn).put(set_apn))
        .route("/ussd", post(ussd))
        .route("/ussd/cancel", post(ussd_cancel))
        .route("/at", post(raw_at))
        .route("/cell", get(cell_info))
        .route("/euicc", get(euicc))
        .route("/euicc/{aid}/profiles/{iccid}/{action}", post(euicc_profile))
        .route(
            "/euicc/{aid}/profiles/{iccid}/nickname",
            axum::routing::put(euicc_nickname),
        )
        .route("/ws", get(ws));
    Router::new()
        .nest("/api", api)
        .fallback(static_file)
        .with_state(state)
}

async fn status(State(m): State<AppState>) -> ApiResult {
    ok(json!({
        "connection": m.connection(),
        "status": m.status(),
        "unread": m.store.unread_count()?,
        "config": {
            "transport": format!("{:?}", m.cfg.transport).to_lowercase(),
            "keepSmsOnSim": m.cfg.keep_sms_on_sim,
            "dataCid": m.cfg.data_cid,
            "pollSecs": m.cfg.poll_secs,
        }
    }))
}

async fn refresh(State(m): State<AppState>) -> ApiResult {
    ok(m.refresh().await?)
}

async fn devices() -> ApiResult {
    ok(crate::transport::scan().await)
}

#[derive(Deserialize)]
struct ListQuery {
    peer: Option<String>,
    before: Option<i64>,
    limit: Option<i64>,
}

async fn list_sms(State(m): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult {
    ok(m.store
        .list(q.peer.as_deref(), q.before, q.limit.unwrap_or(100).clamp(1, 500))?)
}

async fn conversations(State(m): State<AppState>) -> ApiResult {
    ok(m.store.conversations()?)
}

#[derive(Deserialize)]
struct SendBody {
    to: String,
    text: String,
}

async fn send_sms(State(m): State<AppState>, Json(b): Json<SendBody>) -> ApiResult {
    ok(m.send_sms(&b.to, &b.text).await?)
}

async fn sync_sms(State(m): State<AppState>) -> ApiResult {
    ok(json!({ "new": m.sync_sms().await? }))
}

async fn sms_settings(State(m): State<AppState>) -> ApiResult {
    ok(m.sms_settings())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SmsSettingsBody {
    keep_on_modem: Option<bool>,
    receive_storage: Option<String>,
}

async fn set_sms_settings(State(m): State<AppState>, Json(b): Json<SmsSettingsBody>) -> ApiResult {
    ok(m.set_sms_settings(b.keep_on_modem, b.receive_storage.as_deref())
        .await?)
}

async fn delete_sms(State(m): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    ok(json!({ "deleted": m.delete_sms(id).await? }))
}

async fn delete_peer(State(m): State<AppState>, Path(peer): Path<String>) -> ApiResult {
    ok(json!({ "deleted": m.delete_conversation(&peer).await? }))
}

async fn mark_read(State(m): State<AppState>, Path(peer): Path<String>) -> ApiResult {
    ok(json!({ "updated": m.store.mark_read(&peer)? }))
}

#[derive(Deserialize)]
struct DataBody {
    enabled: bool,
}

async fn set_data(State(m): State<AppState>, Json(b): Json<DataBody>) -> ApiResult {
    ok(m.set_data(b.enabled).await?)
}

async fn reset_counters(State(m): State<AppState>) -> ApiResult {
    ok(m.reset_counters().await?)
}

#[derive(Deserialize)]
struct RadioBody {
    mode: RadioMode,
}

async fn set_radio(State(m): State<AppState>, Json(b): Json<RadioBody>) -> ApiResult {
    ok(m.set_radio(b.mode).await?)
}

async fn reboot(State(m): State<AppState>) -> ApiResult {
    m.reboot().await?;
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct PinBody {
    pin: String,
}

async fn enter_pin(State(m): State<AppState>, Json(b): Json<PinBody>) -> ApiResult {
    ok(m.enter_pin(&b.pin).await?)
}

#[derive(Deserialize)]
struct MsisdnBody {
    number: String,
}

async fn set_msisdn(State(m): State<AppState>, Json(b): Json<MsisdnBody>) -> ApiResult {
    ok(m.set_msisdn(&b.number).await?)
}

async fn get_apn(State(m): State<AppState>) -> ApiResult {
    ok(m.pdp_contexts().await?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApnBody {
    cid: Option<u8>,
    pdp_type: String,
    apn: String,
    username: Option<String>,
    password: Option<String>,
    /// 0 none, 1 PAP, 2 CHAP
    auth: Option<u8>,
}

async fn set_apn(State(m): State<AppState>, Json(b): Json<ApnBody>) -> ApiResult {
    let auth = match (&b.username, &b.password) {
        (Some(u), p) if !u.is_empty() => Some((u.as_str(), p.as_deref().unwrap_or(""), b.auth.unwrap_or(1))),
        _ => None,
    };
    let cid = b.cid.unwrap_or(m.cfg.data_cid);
    ok(m.set_apn(cid, &b.pdp_type, &b.apn, auth).await?)
}

#[derive(Deserialize)]
struct UssdBody {
    code: String,
}

async fn ussd(State(m): State<AppState>, Json(b): Json<UssdBody>) -> ApiResult {
    ok(m.ussd(&b.code).await?)
}

async fn ussd_cancel(State(m): State<AppState>) -> ApiResult {
    m.ussd_cancel().await?;
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtBody {
    command: String,
    timeout_ms: Option<u64>,
}

async fn raw_at(State(m): State<AppState>, Json(b): Json<AtBody>) -> ApiResult {
    let timeout = Duration::from_millis(b.timeout_ms.unwrap_or(5000).clamp(100, 180_000));
    ok(m.raw(&b.command, timeout).await?)
}

async fn cell_info(State(m): State<AppState>) -> ApiResult {
    ok(m.cell_info().await?)
}

async fn euicc(State(m): State<AppState>) -> ApiResult {
    ok(m.euicc().await?)
}

async fn euicc_profile(
    State(m): State<AppState>,
    Path((aid, iccid, action)): Path<(String, String, String)>,
) -> ApiResult {
    let action = match action.as_str() {
        "enable" => ProfileAction::Enable,
        "disable" => ProfileAction::Disable,
        "delete" => ProfileAction::Delete,
        _ => return Err(anyhow::anyhow!("unknown action {action}").into()),
    };
    ok(m.euicc_profile(&aid, &iccid, action).await?)
}

#[derive(Deserialize)]
struct NicknameBody {
    nickname: String,
}

async fn euicc_nickname(
    State(m): State<AppState>,
    Path((aid, iccid)): Path<(String, String)>,
    Json(b): Json<NicknameBody>,
) -> ApiResult {
    ok(m.euicc_nickname(&aid, &iccid, &b.nickname).await?)
}

async fn ws(State(m): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| ws_session(m, socket))
}

async fn ws_session(m: AppState, mut socket: WebSocket) {
    let mut rx = m.events.subscribe();
    let hello = [
        Event::Connection {
            connection: m.connection(),
        },
        Event::Status { status: m.status() },
    ];
    for ev in hello {
        let Ok(text) = serde_json::to_string(&ev) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let Ok(text) = serde_json::to_string(&ev) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            }
        }
    }
}

#[derive(RustEmbed)]
#[folder = "../web/dist"]
#[allow_missing = true]
struct Assets;

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (file, name) = match Assets::get(path) {
        Some(f) => (f, path),
        None if !path.starts_with("api/") => match Assets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "frontend not built: run `pnpm build` in web/ and rebuild the server",
                )
                    .into_response();
            }
        },
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let cache = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref().to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        file.data,
    )
        .into_response()
}
