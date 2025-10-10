use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use uuid::Uuid;

const CHAIN_ID_DEFAULT: i64 = 137;
const CHAIN1_ID: i64 = 1;
const CHAIN1_HERMES: &str = "0xa62a2a75949d25e17c6f08a7818e7be97c18a8d2";
const CHAIN2_ID: i64 = 137;
const CHAIN2_HERMES: &str = "0x80ed28d84792d8b153bf2f25f0c4b7a1381de4ab";

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<RwLock<InnerState>>,
    start: Instant,
}

struct InnerState {
    terms_consumer_agreed: bool,
    terms_provider_agreed: bool,
    terms_version: String,
    chain_id: i64,
    chain1_chain_id: i64,
    chain1_hermes: String,
    chain2_chain_id: i64,
    chain2_hermes: String,
    identities: HashSet<String>,
    current_identity: Option<String>,
    connections: HashMap<i32, ConnectionRecord>,
}

struct ConnectionRecord {
    consumer_id: String,
    provider_id: String,
    hermes_id: String,
    session_id: String,
}

#[derive(Clone)]
pub struct ConfigSnapshot {
    pub terms_consumer_agreed: bool,
    pub terms_provider_agreed: bool,
    pub terms_version: String,
    pub chain_id: i64,
    pub chain1_chain_id: i64,
    pub chain1_hermes: String,
    pub chain2_chain_id: i64,
    pub chain2_hermes: String,
}

#[derive(Clone)]
pub struct ConnectionSnapshot {
    pub status: ConnectionStatus,
    pub consumer_id: Option<String>,
    pub provider_id: Option<String>,
    pub hermes_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    NotConnected,
    Connected,
}

impl SharedState {
    pub fn new(terms_version: String) -> Self {
        let inner = InnerState {
            terms_consumer_agreed: false,
            terms_provider_agreed: false,
            terms_version,
            chain_id: CHAIN_ID_DEFAULT,
            chain1_chain_id: CHAIN1_ID,
            chain1_hermes: CHAIN1_HERMES.to_string(),
            chain2_chain_id: CHAIN2_ID,
            chain2_hermes: CHAIN2_HERMES.to_string(),
            identities: HashSet::new(),
            current_identity: None,
            connections: HashMap::new(),
        };

        Self {
            inner: Arc::new(RwLock::new(inner)),
            start: Instant::now(),
        }
    }

    pub fn uptime(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn config_snapshot(&self) -> ConfigSnapshot {
        let inner = self.inner.read();
        ConfigSnapshot {
            terms_consumer_agreed: inner.terms_consumer_agreed,
            terms_provider_agreed: inner.terms_provider_agreed,
            terms_version: inner.terms_version.clone(),
            chain_id: inner.chain_id,
            chain1_chain_id: inner.chain1_chain_id,
            chain1_hermes: inner.chain1_hermes.clone(),
            chain2_chain_id: inner.chain2_chain_id,
            chain2_hermes: inner.chain2_hermes.clone(),
        }
    }

    pub fn update_terms(
        &self,
        consumer: Option<bool>,
        provider: Option<bool>,
        version: Option<String>,
    ) {
        let mut inner = self.inner.write();
        if let Some(value) = consumer {
            inner.terms_consumer_agreed = value;
        }
        if let Some(value) = provider {
            inner.terms_provider_agreed = value;
        }
        if let Some(version) = version {
            inner.terms_version = version;
        }
    }

    pub fn import_identity(&self, address: String, keystore: String) {
        let mut inner = self.inner.write();
        let _ = keystore;
        inner.identities.insert(address);
    }

    pub fn current_identity(&self, requested: Option<String>) -> Option<String> {
        let mut inner = self.inner.write();
        if let Some(id) = requested.filter(|value| !value.is_empty()) {
            if inner.identities.contains(&id) {
                inner.current_identity = Some(id.clone());
                return Some(id);
            }
        }

        if let Some(id) = inner.current_identity.clone() {
            return Some(id);
        }

        inner.identities.iter().next().cloned().map(|id| {
            inner.current_identity = Some(id.clone());
            id
        })
    }

    pub fn identity_exists(&self, address: &str) -> bool {
        self.inner.read().identities.contains(address)
    }

    pub fn connection_status(&self, port: i32) -> ConnectionSnapshot {
        let inner = self.inner.read();
        inner
            .connections
            .get(&port)
            .map(|record| ConnectionSnapshot {
                status: ConnectionStatus::Connected,
                consumer_id: Some(record.consumer_id.clone()),
                provider_id: Some(record.provider_id.clone()),
                hermes_id: Some(record.hermes_id.clone()),
                session_id: Some(record.session_id.clone()),
            })
            .unwrap_or(ConnectionSnapshot {
                status: ConnectionStatus::NotConnected,
                consumer_id: None,
                provider_id: None,
                hermes_id: None,
                session_id: None,
            })
    }

    pub fn create_connection(
        &self,
        port: i32,
        consumer_id: String,
        provider_id: String,
        hermes_id: String,
        _service_type: String,
    ) -> ConnectionSnapshot {
        let mut inner = self.inner.write();
        let session_id = Uuid::new_v4().to_string();
        let record = ConnectionRecord {
            consumer_id: consumer_id.clone(),
            provider_id: provider_id.clone(),
            hermes_id: hermes_id.clone(),
            session_id: session_id.clone(),
        };
        inner.connections.insert(port, record);
        ConnectionSnapshot {
            status: ConnectionStatus::Connected,
            consumer_id: Some(consumer_id),
            provider_id: Some(provider_id),
            hermes_id: Some(hermes_id),
            session_id: Some(session_id),
        }
    }
}
