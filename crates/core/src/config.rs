use crate::monitor::Process;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub refresh_ms: u64,
    pub plugin_timeout_ms: u64,
    pub plugins: Vec<PluginConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_ms: 1000,
            plugin_timeout_ms: 5000,
            plugins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub name: String,
    pub protocol_version: u32,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub match_executables: Vec<String>,
}

impl PluginConfig {
    pub fn matches(&self, process: &Process) -> bool {
        let executable = process
            .executable
            .as_deref()
            .or_else(|| process.command.split_whitespace().next())
            .and_then(|s| Path::new(s).file_name())
            .and_then(|s| s.to_str());
        self.match_executables
            .iter()
            .any(|rule| rule == &process.name || executable == Some(rule.as_str()))
    }
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("top-dog/config.toml"))
    }

    pub fn load(path: Option<&Path>) -> Result<Self, String> {
        let default = Self::default_path();
        let Some(filename) = path.or(default.as_deref()) else {
            return Ok(Self::default());
        };
        let text = match fs::read_to_string(filename) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && path.is_none() => {
                return Ok(Self::default());
            }
            Err(e) => return Err(format!("{}: {e}", filename.display())),
        };
        Self::parse(&text, filename.parent().unwrap_or(Path::new(".")))
    }

    pub fn parse(text: &str, directory: &Path) -> Result<Self, String> {
        let mut config: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if !(100..=60_000).contains(&config.refresh_ms) {
            return Err("refresh_ms must be between 100 and 60000".into());
        }
        if !(100..=30_000).contains(&config.plugin_timeout_ms) {
            return Err("plugin_timeout_ms must be between 100 and 30000".into());
        }
        let mut names = HashSet::new();
        for plugin in &mut config.plugins {
            if plugin.name.is_empty() || !names.insert(plugin.name.clone()) {
                return Err("plugin names must be nonempty and unique".into());
            }
            if plugin.protocol_version != top_dog_protocol::VERSION {
                return Err(format!("{}: unsupported protocol version", plugin.name));
            }
            if plugin.executable.as_os_str().is_empty() || plugin.match_executables.is_empty() {
                return Err("plugins need an executable and match_executables".into());
            }
            if plugin.executable.is_relative() {
                plugin.executable = directory.join(&plugin.executable);
            }
        }
        Ok(config)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.plugin_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_relative_plugin_paths_and_rejects_unknown_fields() {
        let c = Config::parse("[[plugins]]\nname='firefox'\nprotocol_version=1\nexecutable='bin/firefox'\nmatch_executables=['firefox']", Path::new("/config")).unwrap();
        assert_eq!(c.plugins[0].executable, Path::new("/config/bin/firefox"));
        assert!(Config::parse("refresh_ms=0", Path::new("/")).is_err());
        assert!(Config::parse("refresh_typo=1000", Path::new("/")).is_err());
    }
    #[test]
    fn matches_exact_executable_names_including_content_processes() {
        let plugin = PluginConfig {
            name: "firefox".into(),
            protocol_version: 1,
            executable: "/bin/plugin".into(),
            args: vec![],
            match_executables: vec!["firefox".into()],
        };
        let p = Process {
            name: "Isolated Web Co".into(),
            command: "/snap/firefox/firefox -contentproc".into(),
            ..Default::default()
        };
        assert!(plugin.matches(&p));
        assert!(!plugin.matches(&Process {
            command: "bash firefox.sh".into(),
            ..Default::default()
        }));
    }
}
