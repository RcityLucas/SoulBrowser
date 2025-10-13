use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("server busy")]
    ServerBusy,
    #[error("invalid call")]
    InvalidCall,
    #[error("internal error")]
    Internal,
}

impl From<SchedulerError> for soulbrowser_core_types::SoulError {
    fn from(value: SchedulerError) -> Self {
        soulbrowser_core_types::SoulError::new(value.to_string())
    }
}

impl SchedulerError {
    pub fn wrap(err: soulbrowser_core_types::SoulError) -> soulbrowser_core_types::SoulError {
        soulbrowser_core_types::SoulError::new(format!("scheduler error: {err}"))
    }
}
