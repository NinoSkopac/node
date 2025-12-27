mod hermes;
mod identity;
mod proxy;

use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use hermes::HermesClient;
use identity::{import_identity, load_identity};
use proxy::TcpProxy;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(version, about = "Rust-powered consumer for Myst connections", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage identities
    Identities {
        #[command(subcommand)]
        command: IdentityCommands,
    },
    /// Manage connections
    Connection {
        #[command(subcommand)]
        command: ConnectionCommands,
    },
}

#[derive(Subcommand, Debug)]
enum IdentityCommands {
    /// Import an existing keystore JSON
    Import {
        /// Name to save the identity under
        name: String,
        /// JSON string of the V3 keystore
        keystore: String,
        /// Password used to decrypt the keystore
        #[arg(short, long)]
        password: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConnectionCommands {
    /// Establish a proxy connection to a provider
    Up {
        /// Provider identifier (address). Used for Hermes lookups and logging.
        provider: String,
        /// Local proxy port to listen on
        #[arg(long, default_value_t = 10000)]
        proxy: u16,
        /// Optional explicit contact endpoint for the provider (host:port)
        #[arg(long)]
        contact: Option<String>,
        /// Identity name to use from the keystore store
        #[arg(long, default_value = "default")]
        identity: String,
        /// Password to decrypt the keystore. Falls back to MYST_PASSWORD env.
        #[arg(long)]
        password: Option<String>,
        /// Hermes base URL. If omitted Hermes checks are skipped.
        #[arg(long)]
        hermes: Option<String>,
        /// Chain id to query Hermes with
        #[arg(long, default_value_t = 2)]
        chain_id: i64,
        /// Remote TCP port for the provider contact if `contact` only contained a host
        #[arg(long, default_value_t = 4050)]
        remote_port: u16,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    match cli.command {
        Commands::Identities { command } => match command {
            IdentityCommands::Import {
                name,
                keystore,
                password,
            } => {
                let pwd = password.or_else(|| std::env::var("MYST_PASSWORD").ok());
                let identity = import_identity(&name, &keystore, pwd.as_deref())?;
                info!(name = name, address = %identity.address_hex(), "Imported identity");
            }
        },
        Commands::Connection { command } => match command {
            ConnectionCommands::Up {
                provider,
                proxy,
                contact,
                identity,
                password,
                hermes,
                chain_id,
                remote_port,
            } => {
                let pwd = password.or_else(|| std::env::var("MYST_PASSWORD").ok());
                let identity = load_identity(&identity, pwd.as_deref())?;
                info!(address = %identity.address_hex(), "Identity unlocked");

                if let Some(url) = hermes {
                    ensure_hermes_ready(&url, chain_id, &provider, &identity.address_hex()).await?;
                } else {
                    warn!("Hermes URL not provided; skipping payment channel checks");
                }

                let remote = resolve_contact(&provider, contact.as_deref(), remote_port)?;
                info!(local_port = proxy, remote = %remote, "Starting TCP proxy");
                let proxy_server = TcpProxy::new(proxy, remote);
                proxy_server
                    .run_until_ctrl_c(Duration::from_secs(1))
                    .await?;
            }
        },
    }

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter).finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to init tracing subscriber");
}

fn resolve_contact(provider: &str, contact: Option<&str>, remote_port: u16) -> Result<String> {
    if let Some(explicit) = contact {
        return Ok(explicit.to_string());
    }

    if provider.contains(':') {
        return Ok(provider.to_string());
    }

    if provider.is_empty() {
        return Err(anyhow!(
            "provider id is empty; specify --contact or host:port"
        ));
    }

    Ok(format!("{}:{}", provider, remote_port))
}

async fn ensure_hermes_ready(
    url: &str,
    chain_id: i64,
    provider: &str,
    consumer: &str,
) -> Result<()> {
    let client = HermesClient::new(url)?;
    match client.fetch_consumer(chain_id, consumer).await {
        Ok(data) => {
            info!(balance = %data.balance, "Hermes consumer record found");
        }
        Err(err) => {
            warn!("Hermes consumer query failed: {err}");
        }
    }

    match client.fetch_provider(chain_id, provider).await {
        Ok(data) => {
            let promise = data
                .latest_promise
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string());
            info!(latest_promise = %promise, "Hermes provider record found");
        }
        Err(err) => warn!("Hermes provider query failed: {err}"),
    }

    Ok(())
}
