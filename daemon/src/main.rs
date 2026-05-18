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
    APSConnection, APSConnectionResource, APSState, ConversationData, IDSNGMIdentity, IDSUser,
    IMClient, LoginDelegate, MADRID_SERVICE, Message, MessageInst, MessageType, NormalMessage,
    OSConfig,
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
    data_dir: PathBuf,
}

struct Runtime {
    client: IMClient,
    sender_handle: String,
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
    #[serde(default)]
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

async fn provision_runtime(
    data_dir: &Path,
    request: ProvisionRequest,
) -> Result<Runtime, String> {
    let services = &[&MADRID_SERVICE];
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

    let apple_id = request.apple_id.clone();
    let password_hash = sha256(request.password.as_bytes()).to_vec();
    let two_factor_code = request.two_factor_code.clone();

    let account = rustpush::AppleAccount::login(
        move || (apple_id.clone(), password_hash.clone()),
        move || two_factor_code.clone(),
        config.get_gsa_config(&*connection.state.read().await, false),
        anisette,
    )
    .await
    .map_err(|error| error.to_string())?;

    let delegates = login_apple_delegates(
        &account,
        None,
        config.as_ref(),
        &[LoginDelegate::IDS],
    )
    .await
    .map_err(|error| error.to_string())?;

    let mut users = vec![authenticate_apple(
        delegates
            .ids
            .ok_or_else(|| "missing IDS delegate".to_string())?,
        config.as_ref(),
    )
    .await
    .map_err(|error| error.to_string())?];

    let identity = IDSNGMIdentity::new().map_err(|error| error.to_string())?;
    register(
        config.as_ref(),
        &*connection.state.read().await,
        services,
        &mut users,
        &identity,
    )
    .await
    .map_err(|error| error.to_string())?;

    let saved = SavedState {
        push: connection.state.read().await.clone(),
        users,
        identity,
    };
    save_plist(&path(data_dir, "config.plist"), &saved)?;

    build_runtime(connection, config, saved, data_dir).await
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

    match provision_runtime(&state.data_dir, request).await {
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
    init_file_keystore(&data_dir);

    let runtime = boot_from_saved_state(&data_dir)
        .await
        .expect("failed to restore saved runtime");

    let app_state = AppState {
        runtime: Arc::new(Mutex::new(runtime)),
        data_dir,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/provision", post(provision))
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
