use std::fmt;

use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::debug;

#[derive(Clone)]
pub struct HermesClient {
    base: reqwest::Url,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct HermesUserInfo {
    #[serde(default)]
    pub balance: String,
    #[serde(rename = "latest_promise", default)]
    pub latest_promise: Option<PromiseInfo>,
}

#[derive(Debug, Deserialize)]
pub struct PromiseInfo {
    #[serde(default)]
    pub amount: String,
    #[serde(default)]
    pub fee: String,
}

impl fmt::Display for PromiseInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "amount={} fee={}", self.amount, self.fee)
    }
}

impl fmt::Display for HermesUserInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(p) = &self.latest_promise {
            write!(f, "balance={} {p}", self.balance)
        } else {
            write!(f, "balance={}", self.balance)
        }
    }
}

impl HermesClient {
    pub fn new(base: &str) -> Result<Self> {
        let base = reqwest::Url::parse(base).context("parse Hermes URL")?;
        let http = reqwest::Client::builder()
            .user_agent("myst-consumer-rs")
            .build()
            .context("build http client")?;
        Ok(Self { base, http })
    }

    pub async fn fetch_consumer(&self, chain_id: i64, consumer: &str) -> Result<HermesUserInfo> {
        let url = self
            .base
            .join(&format!("data/consumer/{consumer}"))
            .context("build consumer url")?;
        self.fetch(url, chain_id).await
    }

    pub async fn fetch_provider(&self, chain_id: i64, provider: &str) -> Result<HermesUserInfo> {
        let url = self
            .base
            .join(&format!("data/provider/{provider}"))
            .context("build provider url")?;
        self.fetch(url, chain_id).await
    }

    async fn fetch(&self, url: reqwest::Url, chain_id: i64) -> Result<HermesUserInfo> {
        debug!("Hermes GET {}", url);
        let resp = self.http.get(url.clone()).send().await?;
        if resp.status() == StatusCode::NOT_FOUND {
            anyhow::bail!("record not found on Hermes")
        }

        let data: serde_json::Value = resp.json().await?;
        let Some(map) = data.as_object() else {
            anyhow::bail!("unexpected Hermes payload: {data:?}");
        };

        let key = chain_id.to_string();
        let Some(entry) = map.get(&key) else {
            anyhow::bail!("Hermes did not return data for chain {chain_id}");
        };

        let parsed: HermesUserInfo = serde_json::from_value(entry.clone())?;
        Ok(parsed)
    }
}
