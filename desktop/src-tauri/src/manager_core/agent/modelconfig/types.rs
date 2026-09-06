use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::canonical::JsonValue;

pub const VERSION: i64 = 1;
pub const MAX_CONFIG_SIZE: usize = 2 << 20;
pub const MAX_REFERENCED_MODELS_PER_AGENT: usize = 1000;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    OpenCode,
    Codex,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "opencode" => Some(Self::OpenCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Claude, Self::OpenCode, Self::Codex]
    }

    pub fn rank(self) -> usize {
        match self {
            Self::Claude => 0,
            Self::OpenCode => 1,
            Self::Codex => 2,
        }
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub version: i64,
    pub claude: Option<ClaudeConfig>,
    pub opencode: Option<OpenCodeConfig>,
    pub codex: Option<CodexConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub model: String,
    pub name: Option<String>,
    pub context: Option<ClaudeContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudeContext {
    OneMillion,
}

impl ClaudeContext {
    pub fn as_str(self) -> &'static str {
        "1m"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeRole {
    pub inherit_primary: bool,
    pub selection: Option<Model>,
}

impl ClaudeRole {
    pub fn inherit() -> Self {
        Self {
            inherit_primary: true,
            selection: None,
        }
    }

    pub fn selected(model: Model) -> Self {
        Self {
            inherit_primary: false,
            selection: Some(model),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeConfig {
    pub primary: Model,
    pub fable: Option<ClaudeRole>,
    pub haiku: ClaudeRole,
    pub sonnet: ClaudeRole,
    pub opus: ClaudeRole,
    pub context_window: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub extra: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenCodeConfig {
    pub default_model: String,
    pub models: HashMap<String, OpenCodeModelConfig>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenCodeModelConfig {
    pub name: Option<String>,
    pub reasoning: Option<bool>,
    pub attachment: Option<bool>,
    pub tool_call: Option<bool>,
    pub temperature: Option<bool>,
    pub limit: Option<Limit>,
    pub modalities: Option<Modalities>,
    pub interleaved: Option<Interleaved>,
    pub options: Option<HashMap<String, JsonValue>>,
    pub variants: Option<HashMap<String, HashMap<String, JsonValue>>>,
    pub extra: Option<HashMap<String, JsonValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub context: i64,
    pub input: Option<i64>,
    pub output: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Interleaved {
    Enabled,
    Field(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodexConfig {
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub verbosity: Option<String>,
    pub context_window: Option<i64>,
    pub auto_compact_token_limit: Option<i64>,
    pub extra: HashMap<String, JsonValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    pub path: String,
    pub rule: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid model config at {}: {}", self.path, self.rule)
    }
}

impl std::error::Error for ValidationError {}

pub fn invalid(path: impl Into<String>, rule: impl Into<String>) -> ValidationError {
    ValidationError {
        path: path.into(),
        rule: rule.into(),
    }
}

pub use VERSION as Version;
