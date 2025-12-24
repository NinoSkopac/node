mod client;
mod config_view;
mod server;
mod state;

use std::ffi::OsString;
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
    #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
    key: Vec<OsString>,
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
                let ImportIdentityArgs { passphrase, key } = import_args;
                let key = key
                    .into_iter()
                    .map(|segment| {
                        segment.into_string().map_err(|value| {
                            anyhow!("identity key segment must be valid UTF-8, but found {value:?}")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let resolved_key =
                    resolve_identity_key(&key).context("failed to parse identity key argument")?;

                let address = client
                    .import_identity(&passphrase, &resolved_key)
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

fn resolve_identity_key(parts: &[String]) -> Result<String> {
    if parts.is_empty() {
        return Err(anyhow!("missing identity key argument"));
    }

    let combined = if parts.len() == 1 {
        parts[0].clone()
    } else {
        rebuild_brace_expanded_key(parts)?
    };

    let path = std::path::Path::new(&combined);
    if path.exists() {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read identity file at {}", path.display()))?;
        Ok(contents.replace("\\\"", "\""))
    } else {
        Ok(combined)
    }
}

fn rebuild_brace_expanded_key(parts: &[String]) -> Result<String> {
    use serde_json::{Map, Value};

    let mut root = Map::new();

    for part in parts {
        let cleaned = part.replace("\\\"", "\"");
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }

        let (path, value) =
            parse_segment(cleaned).with_context(|| format!("failed to decode segment '{part}'"))?;
        merge_path(&mut root, &path, value);
    }

    Ok(Value::Object(root).to_string())
}

fn parse_segment(segment: &str) -> Result<(Vec<String>, serde_json::Value)> {
    let mut trimmed = segment.trim();
    while trimmed.starts_with('{') {
        trimmed = trimmed[1..].trim_start();
    }
    while trimmed.ends_with('}') {
        trimmed = trimmed[..trimmed.len() - 1].trim_end();
    }

    if trimmed.is_empty() {
        return Err(anyhow!("segment missing key"));
    }

    let (path_part, value) = split_segment(trimmed)?;

    let mut keys = Vec::new();
    let mut remainder = path_part.trim();
    while !remainder.is_empty() {
        while remainder.starts_with('{') {
            remainder = remainder[1..].trim_start();
        }
        while remainder.ends_with('}') {
            remainder = remainder[..remainder.len() - 1].trim_end();
        }

        if remainder.is_empty() {
            break;
        }

        let (key, rest_after_key) =
            parse_json_string(remainder).context("failed to parse segment key component")?;
        keys.push(key);
        remainder = rest_after_key.trim_start();

        if remainder.starts_with(':') {
            remainder = remainder[1..].trim_start();
            continue;
        }

        if !remainder.is_empty() {
            return Err(anyhow!("unexpected characters after key component"));
        }
    }

    if keys.is_empty() {
        return Err(anyhow!("segment missing key path"));
    }

    Ok((keys, value))
}

fn split_segment(segment: &str) -> Result<(&str, serde_json::Value)> {
    use serde_json::Value;

    let trimmed = segment.trim();
    for (idx, ch) in trimmed.char_indices() {
        if matches!(ch, '"' | '{' | '[' | 't' | 'f' | 'n' | '-' | '0'..='9') {
            if let Ok(value) = serde_json::from_str::<Value>(&trimmed[idx..]) {
                let path = trimmed[..idx].trim_end();
                let path = path.trim_end_matches(':').trim_end();
                return Ok((path, value));
            }
        }
    }

    Err(anyhow!("failed to split segment into path and value"))
}

fn parse_json_string(input: &str) -> Result<(String, &str)> {
    if !input.starts_with('"') {
        return Err(anyhow!("expected string literal"));
    }

    let mut escaped = false;
    let mut end_idx = None;
    for (idx, ch) in input.char_indices().skip(1) {
        match ch {
            '"' if !escaped => {
                end_idx = Some(idx + 1);
                break;
            }
            '\\' if !escaped => escaped = true,
            _ => escaped = false,
        }
    }

    let end = end_idx.ok_or_else(|| anyhow!("unterminated string literal"))?;
    let value = serde_json::from_str::<String>(&input[..end])
        .context("failed to deserialize string literal")?;
    Ok((value, &input[end..]))
}

fn merge_path(
    target: &mut serde_json::Map<String, serde_json::Value>,
    path: &[String],
    value: serde_json::Value,
) {
    if let Some((first, rest)) = path.split_first() {
        if rest.is_empty() {
            merge_entry(target, first.clone(), value);
            return;
        }

        let entry = target
            .entry(first.clone())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

        if !entry.is_object() {
            *entry = serde_json::Value::Object(serde_json::Map::new());
        }

        if let Some(map) = entry.as_object_mut() {
            merge_path(map, rest, value);
        }
    }
}

fn merge_entry(
    target: &mut serde_json::Map<String, serde_json::Value>,
    key: String,
    value: serde_json::Value,
) {
    match target.entry(key) {
        serde_json::map::Entry::Vacant(entry) => {
            entry.insert(value);
        }
        serde_json::map::Entry::Occupied(mut entry) => {
            merge_values(entry.get_mut(), value);
        }
    }
}

fn merge_values(existing: &mut serde_json::Value, new_value: serde_json::Value) {
    if let serde_json::Value::Object(existing_map) = existing {
        if let serde_json::Value::Object(new_map) = new_value {
            for (key, value) in new_map.into_iter() {
                merge_entry(existing_map, key, value);
            }
            return;
        }
    }

    *existing = new_value;
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

    #[test]
    fn rebuilds_identity_from_brace_expanded_arguments() {
        let parts = escaped_brace_segments();

        let rebuilt = rebuild_brace_expanded_key(&parts).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rebuilt).unwrap();

        assert_eq!(value["address"], "d363ef3c06eb95460f209e6b8506e103852f75fd");
        assert_eq!(value["crypto"]["cipher"], "aes-128-ctr");
        assert_eq!(value["crypto"]["kdfparams"]["n"], 4096);
        assert_eq!(value["id"], "c8bb6fde-6310-4227-b8f6-59020dc36769");
    }

    #[test]
    fn rebuilds_identity_from_realistic_cli_arguments() {
        let parts = plain_brace_segments();

        let rebuilt = rebuild_brace_expanded_key(&parts).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rebuilt).unwrap();

        assert_eq!(value["address"], "d363ef3c06eb95460f209e6b8506e103852f75fd");
        assert_eq!(value["crypto"]["cipher"], "aes-128-ctr");
        assert_eq!(value["crypto"]["kdfparams"]["n"], 4096);
        assert_eq!(value["id"], "c8bb6fde-6310-4227-b8f6-59020dc36769");
    }

    #[test]
    fn cli_accepts_multiple_identity_key_segments() {
        let args = MystCli::try_parse_from([
            "myst",
            "cli",
            "identities",
            "import",
            "secret",
            r#""address":"0xabc""#,
            r#""crypto":"cipher":"aes-128-ctr""#,
            r#""version":3"#,
        ])
        .unwrap();

        let MystCli {
            command: MystCommand::Cli(CliArgs { subcommand, .. }),
            ..
        } = args
        else {
            panic!("expected cli command");
        };

        let CliSubcommand::Identities(IdentitiesCommand::Import(ImportIdentityArgs {
            passphrase,
            key,
        })) = subcommand.expect("missing identities subcommand");

        let key = key
            .into_iter()
            .map(|value| value.into_string().expect("utf-8 key"))
            .collect::<Vec<_>>();

        assert_eq!(passphrase, "secret");
        assert_eq!(key.len(), 3);
        assert_eq!(key[0], r#""address":"0xabc""#);
    }

    #[test]
    fn resolve_identity_key_falls_back_to_single_argument() {
        let key = String::from("{\"address\":\"0xabc\"}");
        let resolved = resolve_identity_key(&[key.clone()]).unwrap();
        assert_eq!(resolved, key);
    }

    fn escaped_brace_segments() -> Vec<String> {
        plain_brace_segments()
            .into_iter()
            .map(|segment| segment.replace('"', "\\\""))
            .collect()
    }

    fn plain_brace_segments() -> Vec<String> {
        vec![
            String::from("\"address\":\"d363ef3c06eb95460f209e6b8506e103852f75fd\""),
            String::from("\"crypto\":\"cipher\":\"aes-128-ctr\""),
            String::from(
                "\"crypto\":\"ciphertext\":\"480e0f41c5010285ed3eb37bf84cd59ca52059a66dd864fa3787ee919fa7e0c8\"",
            ),
            String::from(
                "\"crypto\":\"cipherparams\":{\"iv\":\"69cbb6f9f0c26a28077b179e874421e5\"}",
            ),
            String::from("\"crypto\":\"kdf\":\"scrypt\""),
            String::from("\"crypto\":\"kdfparams\":\"dklen\":32"),
            String::from("\"crypto\":\"kdfparams\":\"n\":4096"),
            String::from("\"crypto\":\"kdfparams\":\"p\":6"),
            String::from("\"crypto\":\"kdfparams\":\"r\":8"),
            String::from(
                "\"crypto\":\"kdfparams\":\"salt\":\"d9de24291d6622d81132a94b3b73aa2bad287b28e338e38de26dde65d477b3ef\"",
            ),
            String::from(
                "\"crypto\":\"mac\":\"b126a20eedff31785434a5f77b2a1c1886a472617280d2549c2df4f09708cd48\"",
            ),
            String::from("\"id\":\"c8bb6fde-6310-4227-b8f6-59020dc36769\""),
            String::from("\"version\":3"),
        ]
    }

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
