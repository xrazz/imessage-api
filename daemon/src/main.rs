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
    authenticate_apple, default_provider, login_apple_delegates, macos::MacOSConfig, register,
    AppleAccount, APSConnection, APSConnectionResource, APSState, ConversationData,
    DefaultAnisetteProvider, IDSNGMIdentity, IDSUser, IMClient, LoginDelegate, LoginState,
    MADRID_SERVICE, Message, MessageInst, MessageType, NormalMessage, OSConfig, VerifyBody,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    runtime: Arc<Mutex<Option<Runtime>>>,
    pending_login: Arc<Mutex<Option<PendingLogin>>>,
    data_dir: PathBuf,
}

struct Runtime {
    client: IMClient,
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

fn path(dir: &Path, file: &str) -> PathBuf {
    dir.join(file)
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

async fn boot_from_saved_state(data_dir: &Path) -> Result<Option<Runtime>, String> {
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

    build_runtime(connection, config, saved, data_dir).await.map(Some)
}

async fn build_runtime(
    connection: APSConnection,
    config: Arc<MacOSConfig>,
    saved: SavedState,
    data_dir: &Path,
) -> Result<Runtime, String> {
    let services = &[&MADRID_SERVICE];
    let state_path = path(data_dir, "config.plist");
    let shared_state = Arc::new(std::sync::Mutex::new(saved.clone()));
    let persisted_state = shared_state.clone();

    let client = IMClient::new(
        connection,
        saved.users,
        saved.identity,
        services,
        path(data_dir, "id_cache.plist"),
        config,
        Box::new(move |updated_users| {
            let mut state = persisted_state.lock().expect("state lock poisoned");
            state.users = updated_users;
            if let Err(error) = save_plist(&state_path, &*state) {
                tracing::error!("failed to persist updated IDS users: {error}");
            }
        }),
    )
    .await;

    let sender_handle = client
        .identity
        .get_handles()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no sender handle available after registration".to_string())?;

    Ok(Runtime {
        client,
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

async fn finish_provision(
    data_dir: &Path,
    mut pending: PendingLogin,
    code: String,
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
        &[&MADRID_SERVICE],
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
    build_runtime(pending.connection, pending.config, saved, data_dir).await
}

async fn request_sms_code(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse>) {
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

    match finish_provision(&state.data_dir, pending, request.two_factor_code).await {
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

    let mut message = MessageInst::new(
        ConversationData {
            participants: vec![payload.to],
            cv_name: None,
            sender_guid: Some(Uuid::new_v4().to_string()),
            after_guid: None,
        },
        &runtime.sender_handle,
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

    let runtime = boot_from_saved_state(&data_dir)
        .await
        .expect("failed to restore saved runtime");

    let app_state = AppState {
        runtime: Arc::new(Mutex::new(runtime)),
        pending_login: Arc::new(Mutex::new(None)),
        data_dir,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/provision", post(provision))
        .route("/provision/sms", post(request_sms_code))
        .route("/provision/complete", post(complete_provision))
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
