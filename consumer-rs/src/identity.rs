use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ethers_core::types::Address;
use ethers_signers::{LocalWallet, Signer};
use serde_json::Value;
use tracing::info;

const STORE_DIR: &str = ".myst";
const IDENTITY_FOLDER: &str = "identities";

#[derive(Clone)]
pub struct Identity {
    #[allow(dead_code)]
    wallet: Option<LocalWallet>,
    address: Address,
    #[allow(dead_code)]
    path: PathBuf,
}

impl Identity {
    pub fn address_hex(&self) -> String {
        format!("0x{:x}", self.address)
    }

    #[allow(dead_code)]
    pub fn wallet(&self) -> Option<&LocalWallet> {
        self.wallet.as_ref()
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn import_identity(
    name: &str,
    keystore_json: &str,
    password: Option<&str>,
) -> Result<Identity> {
    let path = keystore_path(name)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("keystore path missing parent"))?;
    fs::create_dir_all(parent).context("create identity directory")?;

    // sanity check: ensure JSON parses
    let parsed: Value =
        serde_json::from_str(keystore_json).context("keystore is not valid JSON")?;
    if !parsed.is_object() {
        return Err(anyhow!("keystore should be a JSON object"));
    }

    fs::write(&path, keystore_json).with_context(|| format!("write keystore to {:?}", path))?;
    info!(path = %path.display(), "keystore saved");

    // verify we can decrypt if a password was provided
    let wallet = match password {
        Some(pwd) => Some(decrypt_wallet(&path, pwd)?),
        None => {
            info!("keystore imported without password check; decryption will be required when connecting");
            None
        }
    };

    let address = extract_address(&parsed, wallet.as_ref())?;

    Ok(Identity {
        wallet,
        address,
        path,
    })
}

pub fn load_identity(name: &str, password: Option<&str>) -> Result<Identity> {
    let path = keystore_path(name)?;
    if !path.exists() {
        return Err(anyhow!(
            "identity `{}` is missing; import it with `myst-consumer-rs identities import`",
            name
        ));
    }

    let pwd = password.ok_or_else(|| anyhow!("keystore password missing"))?;
    let wallet = decrypt_wallet(&path, pwd)?;
    let address = wallet.address();
    Ok(Identity {
        wallet: Some(wallet),
        address,
        path,
    })
}

fn decrypt_wallet(path: &Path, password: &str) -> Result<LocalWallet> {
    LocalWallet::decrypt_keystore(path, password)
        .with_context(|| format!("decrypt keystore at {:?}", path))
}

fn keystore_path(name: &str) -> Result<PathBuf> {
    let mut dir = dirs::home_dir().ok_or_else(|| anyhow!("home directory is not set"))?;
    dir.push(STORE_DIR);
    dir.push(IDENTITY_FOLDER);
    dir.push(format!("{name}.json"));
    Ok(dir)
}

fn extract_address(parsed: &Value, wallet: Option<&LocalWallet>) -> Result<Address> {
    if let Some(w) = wallet {
        return Ok(w.address());
    }

    let address = parsed
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow!("keystore is missing `address` field; supply a password so it can be derived")
        })?;

    let hex = address.strip_prefix("0x").unwrap_or(address);
    Ok(Address::from_slice(
        &hex::decode(hex).context("decode keystore address")?,
    ))
}
