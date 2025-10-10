mod client;
mod config_view;
mod server;
mod state;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{ArgAction, Parser, Subcommand};
use client::TequilapiClient;
use config_view::RemoteConfigView;
use server::ServerConfig;

const TERMS_VERSION: &str = "0.0.53";

#[derive(Parser, Debug)]
#[command(
    name = "myst",
    version,
    about = "Minimal Mysterium consumer rewritten in Rust"
)]
struct MystCli {
    #[arg(long = "config-dir")]
    _config_dir: Option<PathBuf>,
    #[arg(long = "script-dir")]
    _script_dir: Option<PathBuf>,
    #[arg(long = "data-dir")]
    _data_dir: Option<PathBuf>,
    #[arg(long = "runtime-dir")]
    _runtime_dir: Option<PathBuf>,
    #[arg(long = "local-service-discovery")]
    _local_service_discovery: Option<bool>,
    #[arg(long = "ui.enable")]
    _ui_enable: Option<bool>,
    #[arg(long = "proxymode", action = ArgAction::SetTrue)]
    _proxymode: bool,
    #[arg(long = "tequilapi.address", default_value = "127.0.0.1")]
    tequilapi_address: String,
    #[arg(long = "tequilapi.allowed-hostnames")]
    _tequilapi_allowed_hostnames: Option<String>,
    #[arg(long = "tequilapi.port", default_value_t = 4050)]
    tequilapi_port: u16,
    #[command(subcommand)]
    command: MystCommand,
}

#[derive(Subcommand, Debug)]
enum MystCommand {
    Daemon,
    Cli(CliArgs),
    #[command(subcommand)]
    Connection(ConnectionSubcommand),
}

#[derive(Parser, Debug)]
struct CliArgs {
    #[arg(long = "agreed-terms-and-conditions", action = ArgAction::SetTrue)]
    agreed_terms: bool,
    #[command(subcommand)]
    subcommand: Option<CliSubcommand>,
}

#[derive(Subcommand, Debug)]
enum CliSubcommand {
    #[command(subcommand)]
    Identities(IdentitiesCommand),
}

#[derive(Subcommand, Debug)]
enum IdentitiesCommand {
    Import(ImportIdentityArgs),
}

#[derive(Parser, Debug)]
struct ImportIdentityArgs {
    passphrase: String,
    key: String,
}

#[derive(Subcommand, Debug)]
enum ConnectionSubcommand {
    Up(ConnectionArgs),
}

#[derive(Parser, Debug)]
struct ConnectionArgs {
    #[arg(long = "agreed-terms-and-conditions", action = ArgAction::SetTrue)]
    agreed_terms: bool,
    #[arg(long = "proxy", default_value_t = 10000)]
    proxy_port: i32,
    #[arg(long = "service-type", default_value = "wireguard")]
    service_type: String,
    #[arg(long = "country")]
    _country: Option<String>,
    #[arg(long = "location-type")]
    _location_type: Option<String>,
    #[arg(long = "sort", default_value = "quality")]
    _sort: String,
    #[arg(long = "include-failed", action = ArgAction::SetTrue)]
    _include_failed: bool,
    provider: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = MystCli::parse();
    let MystCli {
        tequilapi_address,
        tequilapi_port,
        command,
        ..
    } = args;

    match command {
        MystCommand::Daemon => run_daemon(&tequilapi_address, tequilapi_port).await?,
        MystCommand::Cli(cli_args) => run_cli(&tequilapi_address, tequilapi_port, cli_args).await?,
        MystCommand::Connection(subcommand) => match subcommand {
            ConnectionSubcommand::Up(conn_args) => {
                run_connection(&tequilapi_address, tequilapi_port, conn_args).await?
            }
        },
    }

    Ok(())
}

async fn run_daemon(address: &str, port: u16) -> Result<()> {
    let address: IpAddr = address
        .parse()
        .with_context(|| format!("invalid address: {address}"))?;
    let config = ServerConfig {
        bind_addr: SocketAddr::from((address, port)),
        terms_version: TERMS_VERSION.to_string(),
    };

    server::run(config).await
}

async fn run_cli(address: &str, port: u16, cli_args: CliArgs) -> Result<()> {
    let client = build_client(address, port)?;
    client.healthcheck().await?;

    if cli_args.agreed_terms {
        client
            .update_terms(true, true, TERMS_VERSION)
            .await
            .context("failed to agree to terms")?;
        println!("Terms of use accepted.");
    }

    if let Some(subcommand) = cli_args.subcommand {
        match subcommand {
            CliSubcommand::Identities(IdentitiesCommand::Import(import_args)) => {
                let address = client
                    .import_identity(&import_args.passphrase, &import_args.key)
                    .await
                    .context("failed to import identity")?;
                println!("Identity imported: {address}");
            }
        }
    }

    Ok(())
}

async fn run_connection(address: &str, port: u16, conn_args: ConnectionArgs) -> Result<()> {
    let client = build_client(address, port)?;
    client.healthcheck().await?;

    let mut config = client.fetch_config().await?;

    if conn_args.agreed_terms {
        client
            .update_terms(true, false, TERMS_VERSION)
            .await
            .context("failed to agree to consumer terms")?;
        config = client.fetch_config().await?;
    }

    ensure_terms(&config)?;

    let status = client
        .connection_status(conn_args.proxy_port)
        .await
        .context("failed to get connection status")?;

    if !status.is_idle() {
        return Err(anyhow!(
            "You can't create a new connection, you're in state '{}'",
            status.status
        ));
    }

    let consumer_id = client
        .current_identity()
        .await
        .context("failed to obtain current identity")?;

    let identity = client
        .identity(&consumer_id)
        .await
        .context("failed to fetch identity status")?;
    if identity.registration_status.to_lowercase() != "registered" {
        return Err(anyhow!(
            "Your identity is not registered, please execute `myst account register` first"
        ));
    }

    let hermes_id = config
        .hermes_id()
        .context("failed to determine hermes id")?;

    let providers = conn_args
        .provider
        .split(',')
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    if providers.is_empty() {
        return Err(anyhow!("provider id is required"));
    }

    client
        .smart_connection_create(
            &consumer_id,
            &hermes_id,
            &conn_args.service_type,
            providers,
            conn_args.proxy_port,
        )
        .await
        .context("failed to create connection")?;

    println!("Connected");
    Ok(())
}

fn ensure_terms(config: &RemoteConfigView) -> Result<()> {
    if !config.get_bool("terms.consumer-agreed") {
        return Err(anyhow!(
            "you must agree with consumer terms of use in order to use this command"
        ));
    }

    let version = config.get_string("terms.version").unwrap_or_default();
    if version != TERMS_VERSION {
        return Err(anyhow!(
            "you've agreed to terms of use version {version}, but version {TERMS_VERSION} is required"
        ));
    }

    Ok(())
}

fn build_client(address: &str, port: u16) -> Result<TequilapiClient> {
    let base_url = format!("http://{address}:{port}");
    TequilapiClient::new(base_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view_with_terms(agreed: bool, version: &str) -> RemoteConfigView {
        let value = json!({
            "terms": {
                "consumer-agreed": agreed,
                "version": version,
            }
        });
        match value {
            serde_json::Value::Object(map) => RemoteConfigView::new(map),
            _ => unreachable!(),
        }
    }

    #[test]
    fn ensure_terms_allows_matching_version() {
        let view = view_with_terms(true, TERMS_VERSION);
        assert!(ensure_terms(&view).is_ok());
    }

    #[test]
    fn ensure_terms_rejects_missing_agreement() {
        let view = view_with_terms(false, TERMS_VERSION);
        let err = ensure_terms(&view).unwrap_err();
        assert!(err
            .to_string()
            .contains("you must agree with consumer terms of use"));
    }

    #[test]
    fn ensure_terms_rejects_mismatched_version() {
        let view = view_with_terms(true, "0.0.1");
        let err = ensure_terms(&view).unwrap_err();
        assert!(err
            .to_string()
            .contains("you've agreed to terms of use version 0.0.1"));
    }
}
