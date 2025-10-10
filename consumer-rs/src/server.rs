use std::net::SocketAddr;

use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use crate::state::{ConfigSnapshot, ConnectionSnapshot, ConnectionStatus, SharedState};

pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub terms_version: String,
}

pub async fn run(config: ServerConfig) -> Result<()> {
    let state = SharedState::new(config.terms_version);

    let app = Router::new()
        .route("/healthcheck", get(healthcheck))
        .route("/config", get(get_config))
        .route("/terms", post(update_terms))
        .route("/identities-import", post(import_identity))
        .route("/identities/current", put(set_current_identity))
        .route("/identities/:id", get(get_identity))
        .route(
            "/connection",
            get(get_connection_status).put(create_connection),
        )
        .with_state(state);

    axum::serve(tokio::net::TcpListener::bind(config.bind_addr).await?, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn healthcheck(State(state): State<SharedState>) -> Json<HealthcheckResponse> {
    let uptime = humantime::format_duration(state.uptime()).to_string();
    Json(HealthcheckResponse {
        uptime,
        process: std::process::id() as i32,
        version: "0.0.1".to_string(),
        build_info: BuildInfo {
            commit: "<unknown>".to_string(),
            branch: "<unknown>".to_string(),
            build_number: "dev-build".to_string(),
        },
    })
}

async fn get_config(State(state): State<SharedState>) -> Json<ConfigResponse> {
    let snapshot = state.config_snapshot();
    Json(ConfigResponse::from(snapshot))
}

async fn update_terms(
    State(state): State<SharedState>,
    Json(payload): Json<TermsPayload>,
) -> StatusCode {
    state.update_terms(
        payload.agreed_consumer,
        payload.agreed_provider,
        payload.agreed_version,
    );
    StatusCode::OK
}

async fn import_identity(
    State(state): State<SharedState>,
    Json(payload): Json<IdentityImportPayload>,
) -> Result<Json<IdentityRefResponse>, Response> {
    let IdentityImportPayload {
        data,
        current_passphrase,
        set_default,
    } = payload;
    let decoded = general_purpose::STANDARD
        .decode(data)
        .map_err(invalid_request)?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).map_err(invalid_request)?;
    let address = value
        .get("address")
        .and_then(|value| value.as_str())
        .ok_or_else(|| invalid_request_message("identity keystore does not contain address"))?;

    let address_string = address.to_string();
    let _ = current_passphrase;
    state.import_identity(
        address_string.clone(),
        String::from_utf8_lossy(&decoded).to_string(),
    );
    if set_default.unwrap_or(false) {
        state.current_identity(Some(address_string.clone()));
    }

    Ok(Json(IdentityRefResponse { id: address_string }))
}

async fn set_current_identity(
    State(state): State<SharedState>,
    Json(payload): Json<IdentityCurrentPayload>,
) -> Result<Json<IdentityRefResponse>, StatusCode> {
    let IdentityCurrentPayload { id, passphrase } = payload;
    let _ = passphrase;
    if let Some(identity) = state.current_identity(id) {
        return Ok(Json(IdentityRefResponse { id: identity }));
    }

    Err(StatusCode::NOT_FOUND)
}

async fn get_identity(
    Path(id): Path<String>,
    State(state): State<SharedState>,
) -> Result<Json<IdentityInfoResponse>, StatusCode> {
    if !state.identity_exists(&id) {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(IdentityInfoResponse {
        id,
        registration_status: "Registered".to_string(),
    }))
}

async fn get_connection_status(
    State(state): State<SharedState>,
    Query(query): Query<ConnectionStatusQuery>,
) -> Json<ConnectionInfoResponse> {
    let port = query.id.unwrap_or_default();
    let snapshot = state.connection_status(port);
    Json(ConnectionInfoResponse::from(snapshot))
}

async fn create_connection(
    State(state): State<SharedState>,
    Json(payload): Json<ConnectionCreatePayload>,
) -> Result<Json<ConnectionInfoResponse>, StatusCode> {
    let ConnectionCreatePayload {
        consumer_id,
        provider_id,
        hermes_id,
        service_type,
        filter,
        connect_options,
    } = payload;

    let proxy_port = connect_options
        .map(|options| options.proxy_port)
        .unwrap_or(0);
    let provider_id = provider_id
        .filter(|value| !value.is_empty())
        .or_else(|| filter.providers.into_iter().find(|value| !value.is_empty()))
        .ok_or(StatusCode::BAD_REQUEST)?;

    let snapshot = state.create_connection(
        proxy_port,
        consumer_id,
        provider_id,
        hermes_id,
        service_type,
    );

    Ok(Json(ConnectionInfoResponse::from(snapshot)))
}

fn invalid_request<E: std::fmt::Display>(err: E) -> Response {
    (StatusCode::BAD_REQUEST, err.to_string()).into_response()
}

fn invalid_request_message(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}

#[derive(Serialize)]
struct HealthcheckResponse {
    uptime: String,
    process: i32,
    version: String,
    #[serde(rename = "build_info")]
    build_info: BuildInfo,
}

#[derive(Serialize)]
struct BuildInfo {
    commit: String,
    branch: String,
    #[serde(rename = "build_number")]
    build_number: String,
}

#[derive(Serialize)]
struct ConfigResponse {
    data: ConfigData,
}

#[derive(Serialize)]
struct ConfigData {
    #[serde(rename = "chain-id")]
    chain_id: i64,
    terms: TermsData,
    chains: std::collections::HashMap<String, ChainData>,
}

#[derive(Serialize)]
struct TermsData {
    #[serde(rename = "consumer-agreed")]
    consumer_agreed: bool,
    #[serde(rename = "provider-agreed")]
    provider_agreed: bool,
    version: String,
}

#[derive(Serialize)]
struct ChainData {
    #[serde(rename = "chainid")]
    chain_id: i64,
    hermes: String,
}

impl From<ConfigSnapshot> for ConfigResponse {
    fn from(snapshot: ConfigSnapshot) -> Self {
        let ConfigSnapshot {
            terms_consumer_agreed,
            terms_provider_agreed,
            terms_version,
            chain_id,
            chain1_chain_id,
            chain1_hermes,
            chain2_chain_id,
            chain2_hermes,
        } = snapshot;

        let mut chains = std::collections::HashMap::new();
        chains.insert(
            "1".to_string(),
            ChainData {
                chain_id: chain1_chain_id,
                hermes: chain1_hermes,
            },
        );
        chains.insert(
            "2".to_string(),
            ChainData {
                chain_id: chain2_chain_id,
                hermes: chain2_hermes,
            },
        );

        let terms = TermsData {
            consumer_agreed: terms_consumer_agreed,
            provider_agreed: terms_provider_agreed,
            version: terms_version,
        };

        Self {
            data: ConfigData {
                chain_id,
                terms,
                chains,
            },
        }
    }
}

#[derive(Deserialize)]
struct TermsPayload {
    #[serde(rename = "agreed_consumer")]
    agreed_consumer: Option<bool>,
    #[serde(rename = "agreed_provider")]
    agreed_provider: Option<bool>,
    #[serde(rename = "agreed_version")]
    agreed_version: Option<String>,
}

#[derive(Deserialize)]
struct IdentityImportPayload {
    data: String,
    #[serde(rename = "current_passphrase")]
    current_passphrase: Option<String>,
    #[serde(rename = "set_default")]
    set_default: Option<bool>,
}

#[derive(Serialize)]
struct IdentityRefResponse {
    #[serde(rename = "id")]
    id: String,
}

#[derive(Deserialize)]
struct IdentityCurrentPayload {
    #[serde(rename = "id")]
    id: Option<String>,
    #[serde(rename = "passphrase")]
    passphrase: Option<String>,
}

#[derive(Serialize)]
struct IdentityInfoResponse {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "registration_status")]
    registration_status: String,
}

#[derive(Deserialize)]
struct ConnectionStatusQuery {
    id: Option<i32>,
}

#[derive(Deserialize)]
struct ConnectionCreatePayload {
    #[serde(rename = "consumer_id")]
    consumer_id: String,
    #[serde(rename = "provider_id")]
    provider_id: Option<String>,
    #[serde(rename = "hermes_id")]
    hermes_id: String,
    #[serde(rename = "service_type")]
    service_type: String,
    filter: ConnectionCreateFilter,
    #[serde(rename = "connect_options")]
    connect_options: Option<ConnectOptionsPayload>,
}

#[derive(Deserialize)]
struct ConnectionCreateFilter {
    providers: Vec<String>,
}

#[derive(Deserialize)]
struct ConnectOptionsPayload {
    #[serde(rename = "proxy_port")]
    proxy_port: i32,
}

#[derive(Serialize)]
struct ConnectionInfoResponse {
    status: String,
    #[serde(rename = "consumer_id", skip_serializing_if = "Option::is_none")]
    consumer_id: Option<String>,
    #[serde(rename = "provider_id", skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    #[serde(rename = "hermes_id", skip_serializing_if = "Option::is_none")]
    hermes_id: Option<String>,
    #[serde(rename = "session_id", skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

impl From<ConnectionSnapshot> for ConnectionInfoResponse {
    fn from(snapshot: ConnectionSnapshot) -> Self {
        let status = match snapshot.status {
            ConnectionStatus::NotConnected => "NotConnected".to_string(),
            ConnectionStatus::Connected => "Connected".to_string(),
        };

        Self {
            status,
            consumer_id: snapshot.consumer_id,
            provider_id: snapshot.provider_id,
            hermes_id: snapshot.hermes_id,
            session_id: snapshot.session_id,
        }
    }
}
