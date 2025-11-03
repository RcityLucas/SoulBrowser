use serde::Serialize;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize)]
pub struct PluginAuditEvent {
    pub plugin: String,
    pub hook: String,
    pub tenant: Option<String>,
    pub tags: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: Option<SystemTime>,
}

pub trait AuditPort: Send + Sync {
    fn record(&self, _event: PluginAuditEvent) {}
}

pub struct NoopAudit;

impl AuditPort for NoopAudit {}

mod time {
    pub mod serde {
        pub mod rfc3339 {
            use serde::{self, Deserialize, Deserializer, Serializer};
            use std::time::{SystemTime, UNIX_EPOCH};

            pub fn serialize<S>(
                value: &Option<SystemTime>,
                serializer: S,
            ) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                match value {
                    Some(ts) => {
                        let secs = ts
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs_f64();
                        serializer.serialize_some(&secs)
                    }
                    None => serializer.serialize_none(),
                }
            }

            #[allow(dead_code)]
            pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
            where
                D: Deserializer<'de>,
            {
                let opt = Option::<f64>::deserialize(deserializer)?;
                Ok(opt.map(|secs| UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)))
            }
        }
    }
}
