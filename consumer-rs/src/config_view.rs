use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

pub struct RemoteConfigView {
    data: Value,
}

impl RemoteConfigView {
    pub fn new(data: Map<String, Value>) -> Self {
        Self {
            data: Value::Object(data),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        let mut current = &self.data;
        for part in key.to_lowercase().split('.') {
            match current {
                Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Value::as_str).map(|s| s.to_string())
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_i64)
    }

    pub fn hermes_id(&self) -> Result<String> {
        let chain_id = self
            .get_i64("chain-id")
            .ok_or_else(|| anyhow!("missing chain id"))?;
        let chain1_id = self
            .get_i64("chains.1.chainid")
            .ok_or_else(|| anyhow!("missing chain 1 id"))?;
        if chain_id == chain1_id {
            return self
                .get_string("chains.1.hermes")
                .ok_or_else(|| anyhow!("missing chain 1 hermes id"));
        }

        let chain2_id = self
            .get_i64("chains.2.chainid")
            .ok_or_else(|| anyhow!("missing chain 2 id"))?;
        if chain_id == chain2_id {
            return self
                .get_string("chains.2.hermes")
                .ok_or_else(|| anyhow!("missing chain 2 hermes id"));
        }

        Err(anyhow!("no hermes specified for chain {chain_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view_from_json(value: Value) -> RemoteConfigView {
        match value {
            Value::Object(map) => RemoteConfigView::new(map),
            other => panic!("expected object, got {other:?}"),
        }
    }

    #[test]
    fn get_helpers_extract_values() {
        let view = view_from_json(json!({
            "terms": {"consumer-agreed": true},
            "chains": {"1": {"chainid": 1, "hermes": "0xabc"}},
        }));

        assert!(view.get_bool("terms.consumer-agreed"));
        assert_eq!(view.get_i64("chains.1.chainid"), Some(1));
        assert_eq!(view.get_string("chains.1.hermes").as_deref(), Some("0xabc"));
    }

    #[test]
    fn hermes_id_prefers_matching_chain() {
        let view = view_from_json(json!({
            "chain-id": 1,
            "chains": {
                "1": {"chainid": 1, "hermes": "0xhermes1"},
                "2": {"chainid": 137, "hermes": "0xhermes2"}
            }
        }));

        assert_eq!(view.hermes_id().unwrap(), "0xhermes1");
    }

    #[test]
    fn hermes_id_errors_when_chain_is_unknown() {
        let view = view_from_json(json!({
            "chain-id": 999,
            "chains": {
                "1": {"chainid": 1, "hermes": "0xhermes1"},
                "2": {"chainid": 137, "hermes": "0xhermes2"}
            }
        }));

        let err = view.hermes_id().unwrap_err();
        assert!(err
            .to_string()
            .contains("no hermes specified for chain 999"));
    }
}
