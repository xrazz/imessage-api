use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use keystore::{
    init_keystore,
    software::{NoEncryptor, SoftwareKeystore},
};
use openssl::sha::sha256;
use rustpush::{
    authenticate_apple, default_provider,
    facetime::{FTClient, FTMessage, FTState, FACETIME_SERVICE, VIDEO_SERVICE},
    login_apple_delegates,
    macos::MacOSConfig,
    register, APSConnection, APSConnectionResource, APSState, AppleAccount, ConversationData,
    DefaultAnisetteProvider, IDSNGMIdentity, IDSUser, IMClient, LoginDelegate, LoginState, Message,
    MessageInst, MessageType, NormalMessage, OSConfig, VerifyBody, MADRID_SERVICE,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    runtime: Arc<Mutex<Option<Runtime>>>,
    pending_login: Arc<Mutex<Option<PendingLogin>>>,
    facetime_events: Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    current_facetime_call_id: Arc<Mutex<Option<String>>>,
    facetime_webhook_url: Option<String>,
    data_dir: PathBuf,
}

struct Runtime {
    client: Arc<IMClient>,
    facetime: Arc<FTClient>,
    sender_handle: String,
}

struct PendingLogin {
    account: AppleAccount<DefaultAnisetteProvider>,
    connection: APSConnection,
    config: Arc<MacOSConfig>,
    sms_verify_body: Option<VerifyBody>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SavedState {
    push: APSState,
    users: Vec<IDSUser>,
    identity: IDSNGMIdentity,
}

#[derive(Debug, Deserialize)]
struct SendRequest {
    to: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct CallRequest {
    to: String,
}

#[derive(Debug, Deserialize)]
struct TargetRequest {
    to: String,
}

#[derive(Debug, Deserialize)]
struct ProvisionRequest {
    apple_id: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct CompleteProvisionRequest {
    two_factor_code: String,
}

#[derive(Debug, Serialize)]
struct ApiResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct CallResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    join_link: Option<String>,
    sender_handle: String,
    recipient_handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FaceTimeEvent {
    id: String,
    received_at_ms: u128,
    event_type: String,
    call_id: Option<String>,
    direction: Option<String>,
    handle: Option<String>,
    participant: Option<u64>,
    ring: Option<bool>,
    join_link: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct FaceTimeEventsResponse {
    ok: bool,
    events: Vec<FaceTimeEvent>,
}

#[derive(Debug, Serialize)]
struct AvailabilityResponse {
    ok: bool,
    available: bool,
    handle: String,
    sender_handle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct HandlesResponse {
    ok: bool,
    handles: Vec<String>,
    phone_handles: Vec<String>,
    default_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn path(dir: &Path, file: &str) -> PathBuf {
    dir.join(file)
}

fn normalize_handle(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("mailto:") || trimmed.starts_with("tel:") {
        return trimmed.to_string();
    }
    if trimmed.contains('@') {
        return format!("mailto:{}", trimmed.to_lowercase());
    }

    let mut normalized = String::new();
    for (index, ch) in trimmed.chars().enumerate() {
        if ch.is_ascii_digit() || (ch == '+' && index == 0) {
            normalized.push(ch);
        }
    }

    if normalized.starts_with('+') {
        format!("tel:{normalized}")
    } else if normalized.len() == 10 {
        format!("tel:+1{normalized}")
    } else if normalized.len() == 11 && normalized.starts_with('1') {
        format!("tel:+{normalized}")
    } else {
        format!("tel:{normalized}")
    }
}

async fn choose_sender_handle(runtime: &Runtime, target: &str) -> String {
    let handles = runtime.client.identity.get_handles().await;
    if target.starts_with("tel:") {
        if let Some(handle) = handles.iter().find(|handle| handle.starts_with("tel:")) {
            return handle.clone();
        }
    }

    if handles.contains(&runtime.sender_handle) {
        return runtime.sender_handle.clone();
    }

    handles
        .into_iter()
        .next()
        .unwrap_or_else(|| runtime.sender_handle.clone())
}

fn load_plist<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    plist::from_file(path).ok()
}

fn save_plist<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    plist::to_file_xml(path, value).map_err(|error| error.to_string())
}

fn init_file_keystore(data_dir: &Path) {
    let keystore_path = path(data_dir, "keystore.plist");
    let state = plist::from_file(&keystore_path).unwrap_or_default();
    init_keystore(SoftwareKeystore {
        state,
        update_state: Box::new(move |state| {
            if let Err(error) = plist::to_file_xml(&keystore_path, state) {
                tracing::error!("failed to persist keystore: {error}");
            }
        }),
        encryptor: NoEncryptor,
    });
}

fn seed_hwconfig_from_env(data_dir: &Path) -> Result<(), String> {
    let hwconfig_path = path(data_dir, "hwconfig.plist");
    if hwconfig_path.exists() {
        return Ok(());
    }

    let Ok(base64_value) = env::var("HWCONFIG_PLIST_BASE64") else {
        return Ok(());
    };

    use base64::{engine::general_purpose::STANDARD, Engine};
    let decoded = STANDARD
        .decode(base64_value)
        .map_err(|error| format!("invalid HWCONFIG_PLIST_BASE64: {error}"))?;
    fs::write(hwconfig_path, decoded).map_err(|error| error.to_string())
}

async fn boot_from_saved_state(
    data_dir: &Path,
    facetime_events: Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    current_facetime_call_id: Arc<Mutex<Option<String>>>,
    facetime_webhook_url: Option<String>,
) -> Result<Option<Runtime>, String> {
    let Some(config) = load_plist::<MacOSConfig>(&path(data_dir, "hwconfig.plist")) else {
        return Ok(None);
    };
    let Some(saved) = load_plist::<SavedState>(&path(data_dir, "config.plist")) else {
        return Ok(None);
    };

    let config = Arc::new(config);
    let (connection, error) =
        APSConnectionResource::new(config.clone(), Some(saved.push.clone())).await;
    if let Some(error) = error {
        tracing::warn!("APS restored with warning: {error}");
    }

    build_runtime(
        connection,
        config,
        saved,
        data_dir,
        facetime_events,
        current_facetime_call_id,
        facetime_webhook_url,
    )
    .await
    .map(Some)
}

async fn build_runtime(
    connection: APSConnection,
    config: Arc<MacOSConfig>,
    saved: SavedState,
    data_dir: &Path,
    facetime_events: Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    current_facetime_call_id: Arc<Mutex<Option<String>>>,
    facetime_webhook_url: Option<String>,
) -> Result<Runtime, String> {
    let services = &[&MADRID_SERVICE, &FACETIME_SERVICE, &VIDEO_SERVICE];
    let state_path = path(data_dir, "config.plist");
    let facetime_path = path(data_dir, "facetime.plist");
    let shared_state = Arc::new(std::sync::Mutex::new(saved.clone()));
    let persisted_state = shared_state.clone();
    let facetime_state: FTState = load_plist(&facetime_path).unwrap_or_default();

    let client = Arc::new(
        IMClient::new(
            connection.clone(),
            saved.users,
            saved.identity,
            services,
            path(data_dir, "id_cache.plist"),
            config.clone(),
            Box::new(move |updated_users| {
                let mut state = persisted_state.lock().expect("state lock poisoned");
                state.users = updated_users;
                if let Err(error) = save_plist(&state_path, &*state) {
                    tracing::error!("failed to persist updated IDS users: {error}");
                }
            }),
        )
        .await,
    );
    let facetime = Arc::new(
        FTClient::new(
            facetime_state,
            Box::new(move |state| {
                if let Err(error) = save_plist(&facetime_path, state) {
                    tracing::error!("failed to persist FaceTime state: {error}");
                }
            }),
            connection.clone(),
            client.identity.clone(),
            config,
        )
        .await,
    );

    let sender_handle = client
        .identity
        .get_handles()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no sender handle available after registration".to_string())?;

    start_facetime_event_watcher(
        connection,
        facetime.clone(),
        facetime_events,
        current_facetime_call_id,
        facetime_webhook_url,
    )
    .await;

    Ok(Runtime {
        client,
        facetime,
        sender_handle,
    })
}

async fn begin_provision(
    data_dir: &Path,
    request: ProvisionRequest,
) -> Result<PendingLogin, String> {
    let config = load_plist::<MacOSConfig>(&path(data_dir, "hwconfig.plist"))
        .ok_or_else(|| "missing hwconfig.plist".to_string())?;
    let config = Arc::new(config);
    let (connection, error) = APSConnectionResource::new(config.clone(), None).await;
    if let Some(error) = error {
        tracing::warn!("APS created with warning: {error}");
    }

    let anisette = default_provider(
        config.get_gsa_config(&*connection.state.read().await, false),
        path(data_dir, "anisette"),
    );
    let mut account = AppleAccount::new_with_anisette(
        config.get_gsa_config(&*connection.state.read().await, false),
        anisette,
    )
    .map_err(|error| error.to_string())?;
    let password_hash = sha256(request.password.as_bytes()).to_vec();
    let login_state = account
        .login_email_pass(&request.apple_id, &password_hash)
        .await
        .map_err(|error| error.to_string())?;

    match login_state {
        LoginState::LoggedIn => Ok(PendingLogin {
            account,
            connection,
            config,
            sms_verify_body: None,
        }),
        LoginState::NeedsDevice2FA => {
            account
                .send_2fa_to_devices()
                .await
                .map_err(|error| error.to_string())?;
            Ok(PendingLogin {
                account,
                connection,
                config,
                sms_verify_body: None,
            })
        }
        other => Err(format!("unsupported login state: {other:?}")),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

async fn safe_facetime_join_link(facetime: &Arc<FTClient>, call_id: &str) -> Option<String> {
    {
        let state = facetime.state.read().await;
        let Some(session) = state.sessions.get(call_id) else {
            return None;
        };
        if session.my_handles.is_empty() {
            return None;
        }
    }

    match facetime.get_session_link(call_id).await {
        Ok(link) => Some(link),
        Err(error) => {
            tracing::warn!("failed to create FaceTime join link for {call_id}: {error}");
            None
        }
    }
}

async fn facetime_event_from_message(
    facetime: &Arc<FTClient>,
    message: FTMessage,
) -> FaceTimeEvent {
    let mut event = FaceTimeEvent {
        id: Uuid::new_v4().to_string(),
        received_at_ms: now_ms(),
        event_type: "facetime.unknown".to_string(),
        call_id: None,
        direction: None,
        handle: None,
        participant: None,
        ring: None,
        join_link: None,
        message: None,
    };

    match message {
        FTMessage::Ring { guid } => {
            event.event_type = "facetime.ring".to_string();
            event.call_id = Some(guid.clone());
            event.direction = Some("incoming".to_string());
            event.join_link = safe_facetime_join_link(facetime, &guid).await;
        }
        FTMessage::JoinEvent {
            guid,
            participant,
            handle,
            ring,
        } => {
            event.event_type = "facetime.join".to_string();
            event.call_id = Some(guid.clone());
            event.direction = Some(if ring { "incoming" } else { "unknown" }.to_string());
            event.handle = Some(handle);
            event.participant = Some(participant);
            event.ring = Some(ring);
            event.join_link = safe_facetime_join_link(facetime, &guid).await;
        }
        FTMessage::Decline { guid } => {
            event.event_type = "facetime.decline".to_string();
            event.call_id = Some(guid);
            event.direction = Some("outgoing".to_string());
        }
        FTMessage::RespondedElsewhere { guid } => {
            event.event_type = "facetime.responded_elsewhere".to_string();
            event.call_id = Some(guid);
        }
        FTMessage::LinkChanged { guid } => {
            event.event_type = "facetime.link_changed".to_string();
            event.call_id = Some(guid.clone());
            event.join_link = safe_facetime_join_link(facetime, &guid).await;
        }
        FTMessage::AddMembers {
            guid,
            members,
            ring,
        } => {
            event.event_type = "facetime.add_members".to_string();
            event.call_id = Some(guid.clone());
            event.ring = Some(ring);
            event.message = Some(
                members
                    .into_iter()
                    .map(|member| member.handle)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            event.join_link = safe_facetime_join_link(facetime, &guid).await;
        }
        FTMessage::RemoveMembers { guid, members } => {
            event.event_type = "facetime.remove_members".to_string();
            event.call_id = Some(guid);
            event.message = Some(
                members
                    .into_iter()
                    .map(|member| member.handle)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        FTMessage::LeaveEvent {
            guid,
            participant,
            handle,
        } => {
            event.event_type = "facetime.leave".to_string();
            event.call_id = Some(guid);
            event.handle = Some(handle);
            event.participant = Some(participant);
        }
        FTMessage::LetMeInRequest(request) => {
            event.event_type = "facetime.let_me_in_request".to_string();
            event.direction = Some("incoming".to_string());
            event.call_id = facetime_session_for_pseud(facetime, &request.pseud).await;
            event.handle = Some(request.requestor);
            event.message = request.nickname;
        }
    }

    event
}

async fn publish_facetime_event(
    events: &Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    webhook_url: &Option<String>,
    event: FaceTimeEvent,
) {
    {
        let mut events = events.lock().await;
        events.push_back(event.clone());
        while events.len() > 100 {
            events.pop_front();
        }
    }

    let Some(webhook_url) = webhook_url.as_ref() else {
        return;
    };

    let response = reqwest::Client::new()
        .post(webhook_url)
        .json(&event)
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => tracing::warn!(
            "FaceTime webhook returned non-success status: {}",
            response.status()
        ),
        Err(error) => tracing::warn!("failed to POST FaceTime webhook: {error}"),
    }
}

async fn latest_facetime_call_id(facetime: &Arc<FTClient>) -> Option<String> {
    let state = facetime.state.read().await;
    state
        .sessions
        .iter()
        .filter_map(|(guid, session)| {
            session
                .start_time
                .map(|start_time| (guid.clone(), start_time))
        })
        .max_by_key(|(_, start_time)| *start_time)
        .map(|(guid, _)| guid)
}

async fn facetime_session_for_pseud(facetime: &Arc<FTClient>, pseud: &str) -> Option<String> {
    let state = facetime.state.read().await;
    state
        .links
        .get(pseud)
        .and_then(|link| link.session_link.clone())
        .filter(|session_link| state.sessions.contains_key(session_link))
}

async fn start_facetime_event_watcher(
    connection: APSConnection,
    facetime: Arc<FTClient>,
    events: Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    current_call_id: Arc<Mutex<Option<String>>>,
    webhook_url: Option<String>,
) {
    let mut receiver = connection.subscribe().await;
    tokio::spawn(async move {
        loop {
            let message =
                match tokio::time::timeout(std::time::Duration::from_secs(30), receiver.recv())
                    .await
                {
                    Ok(Ok(message)) => message,
                    Ok(Err(error)) => {
                        tracing::warn!(
                            "FaceTime event watcher receiver error: {error}; resubscribing"
                        );
                        receiver = connection.subscribe().await;
                        continue;
                    }
                    Err(error) => {
                        tracing::debug!(
                            "FaceTime event watcher idle timeout: {error}; refreshing subscription"
                        );
                        receiver = connection.subscribe().await;
                        continue;
                    }
                };

            match facetime.handle(message).await {
                Ok(Some(message)) => {
                    match &message {
                        FTMessage::Ring { guid }
                        | FTMessage::LinkChanged { guid }
                        | FTMessage::JoinEvent {
                            guid, ring: true, ..
                        }
                        | FTMessage::AddMembers {
                            guid, ring: true, ..
                        } => {
                            *current_call_id.lock().await = Some(guid.clone());
                        }
                        FTMessage::LetMeInRequest(request) => {
                            let approved_group =
                                match facetime_session_for_pseud(&facetime, &request.pseud).await {
                                    Some(group) => Some(group),
                                    None => {
                                        let current = current_call_id.lock().await.clone();
                                        match current {
                                            Some(group) => Some(group),
                                            None => latest_facetime_call_id(&facetime).await,
                                        }
                                    }
                                };

                            match facetime
                                .respond_letmein(request.clone(), approved_group.as_deref())
                                .await
                            {
                                Ok(()) => {
                                    let join_link = if let Some(group) = approved_group.as_ref() {
                                        safe_facetime_join_link(&facetime, group).await
                                    } else {
                                        None
                                    };
                                    let event = FaceTimeEvent {
                                        id: Uuid::new_v4().to_string(),
                                        received_at_ms: now_ms(),
                                        event_type: "facetime.let_me_in_auto_approved".to_string(),
                                        call_id: approved_group.clone(),
                                        direction: Some("incoming".to_string()),
                                        handle: Some(request.requestor.clone()),
                                        participant: None,
                                        ring: None,
                                        join_link,
                                        message: request.nickname.clone(),
                                    };
                                    publish_facetime_event(&events, &webhook_url, event).await;
                                }
                                Err(error) => {
                                    let event = FaceTimeEvent {
                                        id: Uuid::new_v4().to_string(),
                                        received_at_ms: now_ms(),
                                        event_type: "facetime.let_me_in_auto_approve_failed"
                                            .to_string(),
                                        call_id: approved_group.clone(),
                                        direction: Some("incoming".to_string()),
                                        handle: Some(request.requestor.clone()),
                                        participant: None,
                                        ring: None,
                                        join_link: None,
                                        message: Some(error.to_string()),
                                    };
                                    publish_facetime_event(&events, &webhook_url, event).await;
                                }
                            }
                        }
                        _ => {}
                    }

                    let event = facetime_event_from_message(&facetime, message).await;
                    publish_facetime_event(&events, &webhook_url, event).await;
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("failed to handle FaceTime APS message: {error}"),
            }
        }
    });
}

async fn finish_provision(
    data_dir: &Path,
    mut pending: PendingLogin,
    code: String,
    facetime_events: Arc<Mutex<VecDeque<FaceTimeEvent>>>,
    current_facetime_call_id: Arc<Mutex<Option<String>>>,
    facetime_webhook_url: Option<String>,
) -> Result<Runtime, String> {
    let login_state = if let Some(body) = pending.sms_verify_body.take() {
        pending
            .account
            .verify_sms_2fa(code, body)
            .await
            .map_err(|error| error.to_string())?
    } else {
        pending
            .account
            .verify_2fa(code)
            .await
            .map_err(|error| error.to_string())?
    };

    match login_state {
        LoginState::LoggedIn => {}
        other => return Err(format!("2FA did not complete login: {other:?}")),
    }

    let delegates = login_apple_delegates(
        &pending.account,
        None,
        pending.config.as_ref(),
        &[LoginDelegate::IDS],
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut users = vec![authenticate_apple(
        delegates
            .ids
            .ok_or_else(|| "missing IDS delegate".to_string())?,
        pending.config.as_ref(),
    )
    .await
    .map_err(|error| error.to_string())?];
    let identity = IDSNGMIdentity::new().map_err(|error| error.to_string())?;
    register(
        pending.config.as_ref(),
        &*pending.connection.state.read().await,
        &[&MADRID_SERVICE, &FACETIME_SERVICE, &VIDEO_SERVICE],
        &mut users,
        &identity,
    )
    .await
    .map_err(|error| error.to_string())?;

    let saved = SavedState {
        push: pending.connection.state.read().await.clone(),
        users,
        identity,
    };
    save_plist(&path(data_dir, "config.plist"), &saved)?;
    build_runtime(
        pending.connection,
        pending.config,
        saved,
        data_dir,
        facetime_events,
        current_facetime_call_id,
        facetime_webhook_url,
    )
    .await
}

async fn request_sms_code(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let mut guard = state.pending_login.lock().await;
    let Some(pending) = guard.as_mut() else {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("no_pending_login"),
                message: Some("start provisioning first".to_string()),
            }),
        );
    };

    let result = async {
        let extras = pending
            .account
            .get_auth_extras()
            .await
            .map_err(|error| error.to_string())?;
        let phone = extras
            .trusted_phone_numbers
            .first()
            .ok_or_else(|| "no trusted phone number available".to_string())?;
        match pending
            .account
            .send_sms_2fa_to_devices(phone.id)
            .await
            .map_err(|error| error.to_string())?
        {
            LoginState::NeedsSMS2FAVerification(body) => {
                pending.sms_verify_body = Some(body);
                Ok(())
            }
            other => Err(format!("SMS 2FA did not start: {other:?}")),
        }
    }
    .await;

    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                message_id: None,
                error: None,
                message: Some("sms_verification_code_sent".to_string()),
            }),
        ),
        Err(message) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("sms_request_failed"),
                message: Some(message),
            }),
        ),
    }
}

async fn health(State(state): State<AppState>) -> Json<ApiResponse> {
    let ready = state.runtime.lock().await.is_some();
    Json(ApiResponse {
        ok: true,
        message_id: None,
        error: None,
        message: Some(if ready {
            "ready".to_string()
        } else {
            "unprovisioned".to_string()
        }),
    })
}

async fn handles(State(state): State<AppState>) -> (StatusCode, Json<HandlesResponse>) {
    let guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HandlesResponse {
                ok: false,
                handles: vec![],
                phone_handles: vec![],
                default_handle: None,
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };

    let handles = runtime.client.identity.get_handles().await;
    let phone_handles = runtime.client.identity.get_my_phone_handles().await;
    let default_handle = if handles.contains(&runtime.sender_handle) {
        Some(runtime.sender_handle.clone())
    } else {
        handles.first().cloned()
    };

    (
        StatusCode::OK,
        Json(HandlesResponse {
            ok: true,
            handles,
            phone_handles,
            default_handle,
            error: None,
            message: None,
        }),
    )
}

async fn list_facetime_events(State(state): State<AppState>) -> Json<FaceTimeEventsResponse> {
    let events = state.facetime_events.lock().await.iter().cloned().collect();

    Json(FaceTimeEventsResponse { ok: true, events })
}

async fn provision(
    State(state): State<AppState>,
    Json(request): Json<ProvisionRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    if request.apple_id.trim().is_empty() || request.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("invalid_request"),
                message: Some("`apple_id` and `password` are required".to_string()),
            }),
        );
    }

    match begin_provision(&state.data_dir, request).await {
        Ok(pending) => {
            *state.pending_login.lock().await = Some(pending);
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    message_id: None,
                    error: None,
                    message: Some("verification_code_sent".to_string()),
                }),
            )
        }
        Err(message) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("provision_failed"),
                message: Some(message),
            }),
        ),
    }
}

async fn complete_provision(
    State(state): State<AppState>,
    Json(request): Json<CompleteProvisionRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let Some(pending) = state.pending_login.lock().await.take() else {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("no_pending_login"),
                message: Some("start provisioning first".to_string()),
            }),
        );
    };

    match finish_provision(
        &state.data_dir,
        pending,
        request.two_factor_code,
        state.facetime_events.clone(),
        state.current_facetime_call_id.clone(),
        state.facetime_webhook_url.clone(),
    )
    .await
    {
        Ok(runtime) => {
            *state.runtime.lock().await = Some(runtime);
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    message_id: None,
                    error: None,
                    message: Some("provisioned".to_string()),
                }),
            )
        }
        Err(message) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("provision_failed"),
                message: Some(message),
            }),
        ),
    }
}

async fn send(
    State(state): State<AppState>,
    Json(payload): Json<SendRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    if payload.to.trim().is_empty() || payload.text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("invalid_request"),
                message: Some("`to` and `text` are required".to_string()),
            }),
        );
    }
    let target = normalize_handle(&payload.to);

    let mut guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_mut() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };
    let sender_handle = choose_sender_handle(runtime, &target).await;

    let mut message = MessageInst::new(
        ConversationData {
            participants: vec![target],
            cv_name: None,
            sender_guid: Some(Uuid::new_v4().to_string()),
            after_guid: None,
        },
        &sender_handle,
        Message::Message(NormalMessage::new(payload.text, MessageType::IMessage)),
    );

    match runtime.client.send(&mut message).await {
        Ok(_job) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                message_id: Some(Uuid::new_v4().to_string()),
                error: None,
                message: None,
            }),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("send_failed"),
                message: Some(error.to_string()),
            }),
        ),
    }
}

async fn facetime_call(
    State(state): State<AppState>,
    Json(payload): Json<CallRequest>,
) -> (StatusCode, Json<CallResponse>) {
    if payload.to.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(CallResponse {
                ok: false,
                call_id: None,
                join_link: None,
                sender_handle: "".to_string(),
                recipient_handle: payload.to,
                error: Some("invalid_request"),
                message: Some("`to` is required".to_string()),
            }),
        );
    }
    let target = normalize_handle(&payload.to);

    let guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(CallResponse {
                ok: false,
                call_id: None,
                join_link: None,
                sender_handle: "".to_string(),
                recipient_handle: target,
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };
    let sender_handle = choose_sender_handle(runtime, &target).await;

    match runtime
        .client
        .identity
        .validate_targets(
            &[target.clone()],
            "com.apple.private.alloy.facetime.multi",
            &sender_handle,
        )
        .await
    {
        Ok(valid_targets) if valid_targets.iter().any(|valid| valid == &target) => {}
        Ok(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(CallResponse {
                    ok: false,
                    call_id: None,
                    join_link: None,
                    sender_handle,
                    recipient_handle: target,
                    error: Some("facetime_unavailable"),
                    message: Some(
                        "recipient is not available for FaceTime from this sender".to_string(),
                    ),
                }),
            );
        }
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(CallResponse {
                    ok: false,
                    call_id: None,
                    join_link: None,
                    sender_handle,
                    recipient_handle: target,
                    error: Some("facetime_availability_failed"),
                    message: Some(error.to_string()),
                }),
            );
        }
    }

    let call_id = Uuid::new_v4().to_string().to_uppercase();
    match runtime
        .facetime
        .create_session(call_id.clone(), sender_handle.clone(), &[target.clone()])
        .await
    {
        Ok(()) => match runtime.facetime.get_session_link(&call_id).await {
            Ok(join_link) => (
                StatusCode::OK,
                Json(CallResponse {
                    ok: true,
                    call_id: Some(call_id),
                    join_link: Some(join_link),
                    sender_handle,
                    recipient_handle: target,
                    error: None,
                    message: Some("facetime_call_started".to_string()),
                }),
            ),
            Err(error) => (
                StatusCode::BAD_GATEWAY,
                Json(CallResponse {
                    ok: false,
                    call_id: Some(call_id),
                    join_link: None,
                    sender_handle,
                    recipient_handle: target,
                    error: Some("facetime_link_failed"),
                    message: Some(error.to_string()),
                }),
            ),
        },
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(CallResponse {
                ok: false,
                call_id: None,
                join_link: None,
                sender_handle,
                recipient_handle: target,
                error: Some("facetime_call_failed"),
                message: Some(error.to_string()),
            }),
        ),
    }
}

async fn availability(
    State(state): State<AppState>,
    Json(payload): Json<TargetRequest>,
) -> (StatusCode, Json<AvailabilityResponse>) {
    if payload.to.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(AvailabilityResponse {
                ok: false,
                available: false,
                handle: payload.to,
                sender_handle: "".to_string(),
                error: Some("invalid_request"),
                message: Some("`to` is required".to_string()),
            }),
        );
    }
    let target = normalize_handle(&payload.to);

    let guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(AvailabilityResponse {
                ok: false,
                available: false,
                handle: target,
                sender_handle: "".to_string(),
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };
    let sender_handle = choose_sender_handle(runtime, &target).await;

    match runtime
        .client
        .identity
        .validate_targets(&[target.clone()], "com.apple.madrid", &sender_handle)
        .await
    {
        Ok(valid_targets) => {
            let available = valid_targets.iter().any(|valid| valid == &target);
            (
                StatusCode::OK,
                Json(AvailabilityResponse {
                    ok: true,
                    available,
                    handle: target,
                    sender_handle,
                    error: None,
                    message: None,
                }),
            )
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(AvailabilityResponse {
                ok: false,
                available: false,
                handle: target,
                sender_handle,
                error: Some("availability_failed"),
                message: Some(error.to_string()),
            }),
        ),
    }
}

async fn clear_cache(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };

    runtime.client.identity.invalidate_id_cache().await;
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            message_id: None,
            error: None,
            message: Some("cache_cleared".to_string()),
        }),
    )
}

async fn reregister(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let guard = state.runtime.lock().await;
    let Some(runtime) = guard.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("not_provisioned"),
                message: Some("daemon needs provisioning first".to_string()),
            }),
        );
    };

    match runtime.client.identity.refresh_now().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                message_id: None,
                error: None,
                message: Some("reregistered".to_string()),
            }),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                message_id: None,
                error: Some("reregister_failed"),
                message: Some(error.to_string()),
            }),
        ),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let data_dir = env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/app/data"));
    fs::create_dir_all(&data_dir).expect("failed to create data dir");
    seed_hwconfig_from_env(&data_dir).expect("failed to seed hwconfig");
    init_file_keystore(&data_dir);

    let facetime_event_queue = Arc::new(Mutex::new(VecDeque::new()));
    let current_facetime_call_id = Arc::new(Mutex::new(None));
    let facetime_webhook_url = env::var("FACETIME_WEBHOOK_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let runtime = boot_from_saved_state(
        &data_dir,
        facetime_event_queue.clone(),
        current_facetime_call_id.clone(),
        facetime_webhook_url.clone(),
    )
    .await
    .expect("failed to restore saved runtime");

    let app_state = AppState {
        runtime: Arc::new(Mutex::new(runtime)),
        pending_login: Arc::new(Mutex::new(None)),
        facetime_events: facetime_event_queue,
        current_facetime_call_id,
        facetime_webhook_url,
        data_dir,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/handles", get(handles))
        .route("/provision", post(provision))
        .route("/provision/sms", post(request_sms_code))
        .route("/provision/complete", post(complete_provision))
        .route("/availability", post(availability))
        .route("/facetime/call", post(facetime_call))
        .route("/facetime/events", get(list_facetime_events))
        .route("/cache/clear", post(clear_cache))
        .route("/reregister", post(reregister))
        .route("/send", post(send))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind daemon listener");
    tracing::info!("daemon listening on {}", addr);
    axum::serve(listener, app).await.expect("daemon failed");
}
