use std::collections::HashMap;

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config_view::RemoteConfigView;

const STATUS_NOT_CONNECTED: &str = "NotConnected";

pub struct TequilapiClient {
    base_url: String,
    http: Client,
}

impl TequilapiClient {
    pub fn new(base_url: String) -> Result<Self> {
        let http = Client::builder().build()?;
        Ok(Self { base_url, http })
    }

    pub async fn healthcheck(&self) -> Result<()> {
        self.http
            .get(format!("{}/healthcheck", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn update_terms(&self, consumer: bool, provider: bool, version: &str) -> Result<()> {
        let body = TermsRequest {
            agreed_consumer: Some(consumer),
            agreed_provider: Some(provider),
            agreed_version: version.to_string(),
        };
        self.http
            .post(format!("{}/terms", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn fetch_config(&self) -> Result<RemoteConfigView> {
        let response = self
            .http
            .get(format!("{}/config", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        let wrapper: ConfigResponse = response.json().await?;
        Ok(RemoteConfigView::new(wrapper.data))
    }

    pub async fn import_identity(&self, passphrase: &str, key: &str) -> Result<String> {
        let payload = IdentityImportRequest {
            data: general_purpose::STANDARD.encode(key.as_bytes()),
            current_passphrase: passphrase.to_string(),
            set_default: true,
        };

        let response = self
            .http
            .post(format!("{}/identities-import", self.base_url))
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let identity: IdentityRef = response.json().await?;
        Ok(identity.id)
    }

    pub async fn current_identity(&self) -> Result<String> {
        let request = IdentityCurrentRequest {
            id: Some(String::new()),
            passphrase: Some(String::new()),
        };
        let response = self
            .http
            .put(format!("{}/identities/current", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let identity: IdentityRef = response.json().await?;
        Ok(identity.id)
    }

    pub async fn identity(&self, address: &str) -> Result<IdentityResponse> {
        let response = self
            .http
            .get(format!("{}/identities/{}", self.base_url, address))
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    pub async fn connection_status(&self, proxy_port: i32) -> Result<ConnectionStatus> {
        let mut query = HashMap::new();
        query.insert("id", proxy_port.to_string());
        let response = self
            .http
            .get(format!("{}/connection", self.base_url))
            .query(&query)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    pub async fn smart_connection_create(
        &self,
        consumer_id: &str,
        hermes_id: &str,
        service_type: &str,
        providers: Vec<String>,
        proxy_port: i32,
    ) -> Result<ConnectionStatus> {
        let filter = ConnectionCreateFilter { providers };
        let options = ConnectOptions {
            kill_switch: false,
            dns: "auto".to_string(),
            proxy_port,
        };
        let payload = ConnectionCreateRequest {
            consumer_id: consumer_id.to_string(),
            provider_id: None,
            hermes_id: hermes_id.to_string(),
            service_type: service_type.to_string(),
            filter,
            connect_options: options,
        };

        let response = self
            .http
            .put(format!("{}/connection", self.base_url))
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

#[derive(Serialize)]
struct TermsRequest {
    #[serde(rename = "agreed_consumer")]
    agreed_consumer: Option<bool>,
    #[serde(rename = "agreed_provider")]
    agreed_provider: Option<bool>,
    #[serde(rename = "agreed_version")]
    agreed_version: String,
}

#[derive(Deserialize)]
struct ConfigResponse {
    data: Map<String, Value>,
}

#[derive(Serialize)]
struct IdentityImportRequest {
    data: String,
    #[serde(rename = "current_passphrase")]
    current_passphrase: String,
    #[serde(rename = "set_default")]
    set_default: bool,
}

#[derive(Serialize)]
struct IdentityCurrentRequest {
    #[serde(rename = "id")]
    id: Option<String>,
    #[serde(rename = "passphrase")]
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct IdentityRef {
    #[serde(rename = "id")]
    id: String,
}

#[derive(Deserialize)]
pub struct IdentityResponse {
    #[serde(rename = "registration_status")]
    pub registration_status: String,
}

#[derive(Serialize)]
struct ConnectionCreateRequest {
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
    connect_options: ConnectOptions,
}

#[derive(Serialize)]
struct ConnectionCreateFilter {
    providers: Vec<String>,
}

#[derive(Serialize)]
struct ConnectOptions {
    #[serde(rename = "kill_switch")]
    kill_switch: bool,
    dns: String,
    #[serde(rename = "proxy_port")]
    proxy_port: i32,
}

#[derive(Deserialize)]
pub struct ConnectionStatus {
    pub status: String,
}

impl ConnectionStatus {
    pub fn is_idle(&self) -> bool {
        self.status == STATUS_NOT_CONNECTED
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;
    use httpmock::prelude::*;
    use serde_json::json;

    #[tokio::test]
    async fn import_identity_encodes_payload() {
        let keystore = r#"{"address":"0xabc"}"#;
        let expected_base64 = general_purpose::STANDARD.encode(keystore.as_bytes());

        let server = MockServer::start_async().await;
        let expected_base64_clone = expected_base64.clone();
        let mock = server
            .mock_async(move |when, then| {
                when.method(POST)
                    .path("/identities-import")
                    .json_body(json!({
                        "data": expected_base64_clone,
                        "current_passphrase": "secret",
                        "set_default": true
                    }));
                then.status(200).json_body(json!({"id": "0xabc"}));
            })
            .await;

        let client = TequilapiClient::new(server.base_url()).unwrap();
        let id = client.import_identity("secret", keystore).await.unwrap();

        assert_eq!(id, "0xabc");
        mock.assert();
    }

    #[tokio::test]
    async fn fetch_config_returns_remote_view() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/config");
                then.status(200).json_body(json!({
                    "data": {
                        "chain-id": 1,
                        "chains": {
                            "1": {"chainid": 1, "hermes": "0xhermes"},
                            "2": {"chainid": 137, "hermes": "0xother"}
                        }
                    }
                }));
            })
            .await;

        let client = TequilapiClient::new(server.base_url()).unwrap();
        let view = client.fetch_config().await.unwrap();

        assert_eq!(view.get_i64("chain-id"), Some(1));
        assert_eq!(view.hermes_id().unwrap(), "0xhermes");
        mock.assert();
    }

    #[tokio::test]
    async fn smart_connection_create_sends_expected_payload() {
        let server = MockServer::start_async().await;
        let expected_payload = json!({
            "consumer_id": "0xconsumer",
            "provider_id": null,
            "hermes_id": "0xhermes",
            "service_type": "wireguard",
            "filter": {"providers": ["0xprovider"]},
            "connect_options": {
                "kill_switch": false,
                "dns": "auto",
                "proxy_port": 10000
            }
        });
        let expected_clone = expected_payload.clone();
        let mock = server
            .mock_async(move |when, then| {
                when.method(PUT)
                    .path("/connection")
                    .json_body(expected_clone);
                then.status(200)
                    .json_body(json!({"status": "NotConnected"}));
            })
            .await;

        let client = TequilapiClient::new(server.base_url()).unwrap();
        let status = client
            .smart_connection_create(
                "0xconsumer",
                "0xhermes",
                "wireguard",
                vec!["0xprovider".to_string()],
                10000,
            )
            .await
            .unwrap();

        assert!(status.is_idle());
        mock.assert();
    }

    #[test]
    fn connection_status_idle_helper() {
        let idle = ConnectionStatus {
            status: STATUS_NOT_CONNECTED.to_string(),
        };
        let connected = ConnectionStatus {
            status: "Connected".to_string(),
        };

        assert!(idle.is_idle());
        assert!(!connected.is_idle());
    }
}
