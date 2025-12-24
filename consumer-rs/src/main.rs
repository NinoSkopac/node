use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use clap::{Args, Parser, Subcommand};
use reqwest::blocking::{Client as HttpClient, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(
    name = "myst",
    about = "Minimal Rust consumer CLI for Mysterium",
    version
)]
struct Cli {
    /// Tequilapi host address
    #[arg(long, default_value = "127.0.0.1")]
    tequilapi_address: String,

    /// Tequilapi port
    #[arg(long, default_value_t = 4050)]
    tequilapi_port: u16,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Consumer-oriented CLI commands
    #[command(subcommand)]
    Cli(CliCommand),
    /// Connection management commands
    #[command(subcommand)]
    Connection(ConnectionCommand),
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Identity management commands
    #[command(subcommand)]
    Identities(IdentitiesCommand),
}

#[derive(Subcommand, Debug)]
enum IdentitiesCommand {
    /// Import an existing identity keystore
    Import(ImportArgs),
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// Passphrase of the keystore
    passphrase: String,
    /// Keystore JSON as string or path to a file containing it
    key: String,

    /// Whether to set the imported identity as default
    #[arg(long, default_value_t = true)]
    set_default: bool,
}

#[derive(Subcommand, Debug)]
enum ConnectionCommand {
    /// Bring a connection up to a provider
    Up(UpArgs),
}

#[derive(Args, Debug)]
struct UpArgs {
    /// Local proxy port to bind
    #[arg(long, default_value_t = 10000)]
    proxy: u16,
    /// Provider ID(s), comma separated
    provider: String,

    /// Service type to request (wireguard/openvpn/noop)
    #[arg(long, default_value = "wireguard")]
    service_type: String,

    /// Optional country filter
    #[arg(long)]
    country: Option<String>,

    /// Optional location type filter
    #[arg(long, value_name = "TYPE")]
    location_type: Option<String>,

    /// Include providers with failed monitoring results
    #[arg(long, default_value_t = false)]
    include_failed: bool,

    /// Sorting strategy for proposals
    #[arg(long, default_value = "quality")]
    sort: String,

    /// DNS selection (auto/provider/system/custom)
    #[arg(long, default_value = "auto")]
    dns: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = TequilapiClient::new(&cli.tequilapi_address, cli.tequilapi_port)?;

    match cli.command {
        Command::Cli(cli_command) => match cli_command {
            CliCommand::Identities(identity_command) => match identity_command {
                IdentitiesCommand::Import(args) => import_identity(&client, args),
            },
        },
        Command::Connection(connection_command) => match connection_command {
            ConnectionCommand::Up(args) => connection_up(&client, args),
        },
    }
}

fn import_identity(client: &TequilapiClient, args: ImportArgs) -> Result<()> {
    let key_data = read_key_blob(&args.key)?;
    let identity = client
        .import_identity(&args.passphrase, &key_data, args.set_default)
        .context("failed to import identity")?;
    println!("Identity imported: {}", identity.address);
    Ok(())
}

fn connection_up(client: &TequilapiClient, args: UpArgs) -> Result<()> {
    let terms = client.get_terms().context("failed to read terms state")?;
    if !terms.agreed_consumer || terms.agreed_version.as_deref() != terms.current_version.as_deref()
    {
        let agreed_version = terms
            .current_version
            .clone()
            .ok_or_else(|| anyhow!("server did not report current terms version"))?;
        client
            .update_terms(true, &agreed_version)
            .context("failed to store terms agreement")?;
    }

    let status = client
        .connection_status(args.proxy)
        .context("failed to read connection status")?;
    if let Some(state) = status.status.as_deref() {
        if matches!(
            state,
            "Connecting" | "Disconnecting" | "Reconnecting" | "Connected"
        ) {
            bail!("Connection already in state '{state}', aborting");
        }
    }

    let identity = client
        .current_identity()
        .context("failed to get current identity")?;
    let identity_status = client
        .identity_status(&identity.address)
        .context("failed to read identity status")?;

    if identity_status
        .registration_status
        .as_deref()
        .map(|s| s.to_ascii_lowercase())
        != Some("registered".into())
    {
        bail!(
            "Identity {} is not registered. Run account registration first.",
            identity.address
        );
    }

    let config = client
        .fetch_config()
        .context("failed to fetch remote config")?;
    let chain_id = lookup_i64(&config, "chain-id").unwrap_or(1);
    let hermes_key = format!("chains.{chain_id}.hermes");
    let hermes_id = lookup_string(&config, &hermes_key)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("could not find hermes id for chain {chain_id}"))?;

    let providers: Vec<String> = args
        .provider
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().to_string())
        .collect();

    let request = ConnectionCreateRequest {
        consumer_id: identity.address,
        provider_id: None,
        filter: ConnectionCreateFilter {
            providers: if providers.is_empty() {
                None
            } else {
                Some(providers)
            },
            country_code: args.country.clone(),
            ip_type: args.location_type.clone(),
            include_monitoring_failed: args.include_failed,
            sort_by: Some(args.sort.clone()),
        },
        hermes_id,
        service_type: args.service_type.clone(),
        connect_options: ConnectOptions {
            disable_kill_switch: false,
            dns: args.dns.clone(),
            proxy_port: args.proxy as i32,
        },
    };

    client
        .create_connection(&request)
        .context("failed to create connection")?;

    println!("Connected");
    Ok(())
}

fn read_key_blob(input: &str) -> Result<Vec<u8>> {
    let path = Path::new(input);
    if path.exists() {
        let raw = fs::read(path).with_context(|| format!("unable to read file {input}"))?;
        let cleaned = String::from_utf8_lossy(&raw).replace("\\\"", "\"");
        return Ok(cleaned.into_bytes());
    }

    Ok(input.as_bytes().to_vec())
}

fn lookup_i64(config: &Value, key: &str) -> Option<i64> {
    lookup_value(config, key).and_then(|v| match v {
        Value::Number(num) => num.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn lookup_string(config: &Value, key: &str) -> Option<String> {
    lookup_value(config, key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    })
}

fn lookup_value<'a>(config: &'a Value, key: &str) -> Option<&'a Value> {
    if let Some(direct) = config.get(key) {
        return Some(direct);
    }

    let mut current = config;
    for segment in key.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(segment)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

struct TequilapiClient {
    base_url: String,
    http: HttpClient,
}

impl TequilapiClient {
    fn new(host: &str, port: u16) -> Result<Self> {
        let base_url = format!("http://{host}:{port}");
        let http = HttpClient::builder()
            .user_agent("myst-consumer-rs/0.1")
            .build()
            .context("failed to construct HTTP client")?;
        Ok(Self { base_url, http })
    }

    fn import_identity(
        &self,
        passphrase: &str,
        blob: &[u8],
        set_default: bool,
    ) -> Result<IdentityRef> {
        let payload = IdentityImportRequest {
            data: BASE64.encode(blob),
            current_passphrase: passphrase.to_string(),
            set_default,
            new_passphrase: String::new(),
        };
        let resp = self
            .http
            .post(format!("{}/identities-import", self.base_url))
            .json(&payload)
            .send()
            .context("request to import identity failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse identity response")?)
    }

    fn current_identity(&self) -> Result<IdentityRef> {
        let payload = IdentityCurrentRequest {
            address: String::new(),
            passphrase: String::new(),
        };
        let resp = self
            .http
            .put(format!("{}/identities/current", self.base_url))
            .json(&payload)
            .send()
            .context("request to fetch current identity failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse identity response")?)
    }

    fn identity_status(&self, address: &str) -> Result<IdentityStatus> {
        let resp = self
            .http
            .get(format!("{}/identities/{address}", self.base_url))
            .send()
            .context("request to read identity status failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse identity status")?)
    }

    fn get_terms(&self) -> Result<TermsResponse> {
        let resp = self
            .http
            .get(format!("{}/terms", self.base_url))
            .send()
            .context("request to fetch terms failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse terms response")?)
    }

    fn update_terms(&self, agreed_consumer: bool, version: &str) -> Result<()> {
        let payload = TermsRequest {
            agreed_consumer,
            agreed_provider: None,
            agreed_version: Some(version.to_string()),
        };
        let resp = self
            .http
            .post(format!("{}/terms", self.base_url))
            .json(&payload)
            .send()
            .context("request to update terms failed")?;
        ensure_success(resp)?;
        Ok(())
    }

    fn fetch_config(&self) -> Result<Value> {
        #[derive(Deserialize)]
        struct ConfigResponse {
            data: Value,
        }

        let resp = self
            .http
            .get(format!("{}/config", self.base_url))
            .send()
            .context("request to fetch config failed")?;
        let resp = ensure_success(resp)?;
        let parsed: ConfigResponse = resp.json().context("failed to parse config response")?;
        Ok(parsed.data)
    }

    fn connection_status(&self, proxy_port: u16) -> Result<ConnectionInfo> {
        let resp = self
            .http
            .get(format!("{}/connection", self.base_url))
            .query(&[("id", proxy_port.to_string())])
            .send()
            .context("request to fetch connection status failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse connection status")?)
    }

    fn create_connection(&self, request: &ConnectionCreateRequest) -> Result<ConnectionInfo> {
        let resp = self
            .http
            .put(format!("{}/connection", self.base_url))
            .json(request)
            .send()
            .context("request to create connection failed")?;
        let resp = ensure_success(resp)?;
        Ok(resp.json().context("failed to parse connection response")?)
    }
}

fn ensure_success(resp: Response) -> Result<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }

    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    Err(anyhow!("request failed with status {status}: {body}"))
}

#[derive(Serialize)]
struct IdentityImportRequest {
    data: String,
    #[serde(rename = "current_passphrase")]
    current_passphrase: String,
    #[serde(rename = "set_default")]
    set_default: bool,
    #[serde(rename = "new_passphrase")]
    new_passphrase: String,
}

#[derive(Serialize)]
struct IdentityCurrentRequest {
    address: String,
    passphrase: String,
}

#[derive(Deserialize)]
struct IdentityRef {
    address: String,
}

#[derive(Deserialize)]
struct IdentityStatus {
    #[serde(rename = "registration_status")]
    registration_status: Option<String>,
}

#[derive(Deserialize)]
struct TermsResponse {
    #[serde(rename = "agreed_provider")]
    _agreed_provider: bool,
    #[serde(rename = "agreed_consumer")]
    agreed_consumer: bool,
    #[serde(rename = "agreed_version")]
    agreed_version: Option<String>,
    #[serde(rename = "current_version")]
    current_version: Option<String>,
}

#[derive(Serialize)]
struct TermsRequest {
    #[serde(rename = "agreed_provider")]
    agreed_provider: Option<bool>,
    #[serde(rename = "agreed_consumer")]
    agreed_consumer: bool,
    #[serde(rename = "agreed_version")]
    agreed_version: Option<String>,
}

#[derive(Serialize)]
struct ConnectionCreateRequest {
    #[serde(rename = "consumer_id")]
    consumer_id: String,
    #[serde(rename = "provider_id", skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    filter: ConnectionCreateFilter,
    #[serde(rename = "hermes_id")]
    hermes_id: String,
    #[serde(rename = "service_type")]
    service_type: String,
    #[serde(rename = "connect_options")]
    connect_options: ConnectOptions,
}

#[derive(Serialize)]
struct ConnectionCreateFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    providers: Option<Vec<String>>,
    #[serde(rename = "country_code", skip_serializing_if = "Option::is_none")]
    country_code: Option<String>,
    #[serde(rename = "ip_type", skip_serializing_if = "Option::is_none")]
    ip_type: Option<String>,
    #[serde(rename = "include_monitoring_failed")]
    include_monitoring_failed: bool,
    #[serde(rename = "sort_by", skip_serializing_if = "Option::is_none")]
    sort_by: Option<String>,
}

#[derive(Serialize)]
struct ConnectOptions {
    #[serde(rename = "kill_switch")]
    disable_kill_switch: bool,
    dns: String,
    #[serde(rename = "proxy_port")]
    proxy_port: i32,
}

#[derive(Deserialize)]
struct ConnectionInfo {
    status: Option<String>,
}
