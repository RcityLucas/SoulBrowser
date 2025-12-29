use std::{collections::HashMap, env, fs, path::PathBuf};

use serde::Deserialize;

use crate::{model::AgentRequest, planner::PlanStageKind};

#[derive(Debug, Clone)]
pub struct PlanStageGraph {
    intents: HashMap<String, IntentStagePlan>,
    default_plan: IntentStagePlan,
}

impl PlanStageGraph {
    pub fn new(intents: HashMap<String, IntentStagePlan>, default_plan: IntentStagePlan) -> Self {
        Self {
            intents,
            default_plan,
        }
    }

    fn load_from_config(config: StagePlanConfigFile) -> Self {
        let default_plan = IntentStagePlan::from_config("default", config.defaults);
        let intents = config
            .intents
            .into_iter()
            .map(|(id, entry)| (id.clone(), IntentStagePlan::from_config(&id, entry)))
            .collect();
        Self::new(intents, default_plan)
    }

    pub fn load_from_env_or_default() -> Option<Self> {
        if let Some(path) = env::var_os("SOUL_PLANNER_STAGE_CONFIG") {
            if let Ok(bytes) = fs::read(PathBuf::from(path)) {
                if let Ok(config) = serde_yaml::from_slice::<StagePlanConfigFile>(&bytes) {
                    return Some(Self::load_from_config(config));
                }
            }
        }

        for path in [
            "config/planner/stage_graph.yaml",
            "config/defaults/stage_graph.yaml",
        ] {
            let candidate = PathBuf::from(path);
            if !candidate.exists() {
                continue;
            }
            if let Ok(bytes) = fs::read(&candidate) {
                if let Ok(config) = serde_yaml::from_slice::<StagePlanConfigFile>(&bytes) {
                    return Some(Self::load_from_config(config));
                }
            }
        }

        None
    }

    pub fn plan_for_request(&self, request: &AgentRequest) -> IntentStagePlan {
        if let Some(intent_id) = request.intent.intent_id.as_deref() {
            if let Some(plan) = self.intents.get(intent_id) {
                return plan.clone();
            }
        }
        let kind_slug = request.intent.intent_kind.as_str();
        if let Some(plan) = self.intents.get(kind_slug) {
            return plan.clone();
        }
        self.default_plan.clone()
    }
}

impl Default for PlanStageGraph {
    fn default() -> Self {
        Self::load_from_config(StagePlanConfigFile::default())
    }
}

#[derive(Debug, Clone)]
pub struct IntentStagePlan {
    pub id: String,
    pub stages: Vec<StageStrategyChain>,
}

impl IntentStagePlan {
    fn from_config(id: &str, config: IntentStageConfig) -> Self {
        let stages = config
            .stages
            .into_iter()
            .filter_map(StageStrategyChain::from_map)
            .collect();
        Self {
            id: id.to_string(),
            stages,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StageStrategyChain {
    pub stage: PlanStageKind,
    pub strategies: Vec<String>,
}

impl StageStrategyChain {
    fn from_map(map: HashMap<String, Vec<String>>) -> Option<Self> {
        let (stage_name, strategies) = map.into_iter().next()?;
        let stage = PlanStageKind::from_str(&stage_name)?;
        let strategies = if strategies.is_empty() {
            vec!["auto".to_string()]
        } else {
            strategies
        };
        Some(Self { stage, strategies })
    }
}

#[derive(Debug, Deserialize)]
struct StagePlanConfigFile {
    #[serde(default)]
    intents: HashMap<String, IntentStageConfig>,
    #[serde(default = "StagePlanConfigFile::default_plan")]
    defaults: IntentStageConfig,
}

impl StagePlanConfigFile {
    fn default_plan() -> IntentStageConfig {
        IntentStageConfig::default()
    }
}

impl Default for StagePlanConfigFile {
    fn default() -> Self {
        Self {
            intents: HashMap::new(),
            defaults: IntentStageConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct IntentStageConfig {
    #[serde(default = "IntentStageConfig::default_stages")]
    stages: Vec<HashMap<String, Vec<String>>>,
}

impl IntentStageConfig {
    fn default_stages() -> Vec<HashMap<String, Vec<String>>> {
        vec![
            stage_entry("navigate", vec!["context_url", "preferred_site", "search"]),
            stage_entry("act", vec!["auto"]),
            stage_entry("observe", vec!["extract_site"]),
            stage_entry("parse", vec!["generic_parser"]),
            stage_entry("deliver", vec!["structured", "agent_note"]),
        ]
    }
}

impl Default for IntentStageConfig {
    fn default() -> Self {
        Self {
            stages: Self::default_stages(),
        }
    }
}

fn stage_entry(stage: &str, strategies: Vec<&str>) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    map.insert(
        stage.to_string(),
        strategies.into_iter().map(|s| s.to_string()).collect(),
    );
    map
}
