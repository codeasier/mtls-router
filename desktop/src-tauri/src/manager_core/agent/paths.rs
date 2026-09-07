use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paths {
    pub config_path: PathBuf,
    pub auth_path: Option<PathBuf>,
}

pub fn claude_paths(home: &Path, config_dir: &str) -> Paths {
    let dir = if config_dir.is_empty() {
        home.join(".claude")
    } else {
        PathBuf::from(config_dir)
    };
    Paths {
        config_path: dir.join("settings.json"),
        auth_path: None,
    }
}

pub fn opencode_paths(home: &Path, config_override: &str) -> Paths {
    if !config_override.is_empty() {
        return Paths {
            config_path: PathBuf::from(config_override),
            auth_path: None,
        };
    }
    let dir = home.join(".config").join("opencode");
    let json_path = dir.join("opencode.json");
    let jsonc_path = dir.join("opencode.jsonc");
    if is_regular_file(&json_path) {
        return Paths {
            config_path: json_path,
            auth_path: None,
        };
    }
    if is_regular_file(&jsonc_path) {
        return Paths {
            config_path: jsonc_path,
            auth_path: None,
        };
    }
    Paths {
        config_path: json_path,
        auth_path: None,
    }
}

pub fn codex_paths(home: &Path, codex_home: &str) -> Paths {
    let dir = if codex_home.is_empty() {
        home.join(".codex")
    } else {
        PathBuf::from(codex_home)
    };
    Paths {
        config_path: dir.join("config.toml"),
        auth_path: Some(dir.join("auth.json")),
    }
}

pub fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|info| info.is_file())
        .unwrap_or(false)
}
