use async_trait::async_trait;
use soulbrowser_core_types::{ExecRoute, SoulError};

use crate::model::InputMode;
use crate::ports::{TempoPort, TypingPlan, TypingStep};

/// No-op tempo provider; real implementations can introduce natural typing.
#[derive(Clone, Debug, Default)]
pub struct NullTempo;

#[async_trait]
impl TempoPort for NullTempo {
    async fn build_plan(&self, mode: InputMode, text: &str) -> Result<TypingPlan, SoulError> {
        match mode {
            InputMode::Natural => Ok(TypingPlan {
                steps: text
                    .chars()
                    .map(|ch| TypingStep {
                        chunk: ch.to_string(),
                        delay_ms: 50,
                    })
                    .collect(),
            }),
            _ => Ok(TypingPlan { steps: Vec::new() }),
        }
    }

    async fn run_plan(&self, _route: &ExecRoute, _plan: &TypingPlan) -> Result<(), SoulError> {
        Ok(())
    }
}
