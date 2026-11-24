use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_SERVER: &str = "https://struggletime.screenseekr.com";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkedProject {
    #[serde(default)]
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default = "default_server")]
    pub server: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_bucket_mins")]
    pub bucket_mins: u32,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub projects: Vec<LinkedProject>,
    /// Dashboard / tray colourscheme id (classic, neon, nord, …).
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_server() -> String {
    DEFAULT_SERVER.to_string()
}
fn default_bucket_mins() -> u32 {
    5
}
fn default_theme() -> String {
    "classic".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            handle: String::new(),
            repo: String::new(),
            server: DEFAULT_SERVER.to_string(),
            token: String::new(),
            bucket_mins: 5,
            project: String::new(),
            projects: vec![],
            theme: default_theme(),
        }
    }
}

pub fn runtime_home() -> PathBuf {
    if let Ok(v) = std::env::var("STRUGGLETIME_HOME").or_else(|_| std::env::var("AITIME_HOME")) {
        return PathBuf::from(v);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aitime")
}

pub fn config_home() -> PathBuf {
    if let Ok(v) = std::env::var("STRUGGLETIME_CONFIG").or_else(|_| std::env::var("AITIME_CONFIG"))
    {
        return PathBuf::from(v);
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("struggletime");
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("struggletime")
}

pub fn settings_path() -> PathBuf {
    config_home().join("settings.json")
}

pub fn daemon_log_path() -> PathBuf {
    runtime_home().join("daemon.log")
}

pub fn daemon_pid_path() -> PathBuf {
    runtime_home().join("daemon.pid")
}

pub fn pause_flag_path() -> PathBuf {
    runtime_home().join("paused")
}

pub fn normalize_server(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() {
        return DEFAULT_SERVER.to_string();
    }
    // Drop ephemeral tunnel hosts — same policy as the settings normalizer.
    let lower = u.to_lowercase();
    if lower.contains("trycloudflare.com")
        || lower.contains("ngrok")
        || lower.contains("loca.lt")
        || lower.contains("cloudflared")
    {
        return DEFAULT_SERVER.to_string();
    }
    u.to_string()
}

pub fn load_settings() -> Result<Settings> {
    let path = settings_path();
    if !path.exists() {
        let legacy = runtime_home().join("gui.json");
        if legacy.exists() {
            let raw = fs::read_to_string(&legacy)?;
            let mut s: Settings = serde_json::from_str(&raw).unwrap_or_default();
            s.server = normalize_server(&s.server);
            let _ = save_settings(&s);
            return Ok(s);
        }
        return Ok(Settings::default());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut s: Settings = serde_json::from_str(&raw).unwrap_or_default();
    s.server = normalize_server(&s.server);
    Ok(s)
}

pub fn save_settings(s: &Settings) -> Result<()> {
    let dir = config_home();
    fs::create_dir_all(&dir)?;
    let mut out = s.clone();
    out.server = normalize_server(&out.server);
    let raw = serde_json::to_string_pretty(&out)?;
    fs::write(settings_path(), raw)?;
    Ok(())
}

pub fn linked_paths(s: &Settings) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for p in &s.projects {
        let path = PathBuf::from(shellexpand_home(&p.path));
        if path.is_dir() {
            out.push((p.id.clone(), path));
        }
    }
    if out.is_empty() && !s.repo.is_empty() {
        let path = PathBuf::from(shellexpand_home(&s.repo));
        if path.is_dir() {
            out.push((s.project.clone(), path));
        }
    }
    out
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkProjectError {
    /// Creating/linking would steal or merge into an existing project id at another path.
    IdCollision {
        id: String,
        existing_path: String,
        new_path: String,
    },
}

impl std::fmt::Display for LinkProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdCollision { id, .. } => write!(
                f,
                "project id '{id}' already exists (linked folder differs from the one you chose). \
                 Pick a unique --project name, or unlink the old project first. \
                 Refusing to silently merge hours across folders."
            ),
        }
    }
}

impl std::error::Error for LinkProjectError {}

/// Link or create a local project folder.
///
/// * `create = true` — new project: fail with [`LinkProjectError::IdCollision`] if `project_id`
///   already exists at a different path. Same path + same id is idempotent.
/// * `create = false` — link: same rules for id collisions (never silently retarget an id).
pub fn link_project(
    s: &mut Settings,
    project_id: &str,
    local_path: &Path,
    _create: bool,
) -> Result<(), LinkProjectError> {
    let path = local_path.to_string_lossy().to_string();
    let id = if project_id.is_empty() {
        local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    } else {
        project_id.to_string()
    };

    if let Some(existing) = s.projects.iter().find(|p| p.id == id) {
        if existing.path != path {
            return Err(LinkProjectError::IdCollision {
                id: id.clone(),
                existing_path: existing.path.clone(),
                new_path: path,
            });
        }
        s.repo = path;
        s.project = id;
        return Ok(());
    }

    if let Some(row) = s.projects.iter_mut().find(|p| p.path == path) {
        // Same folder, new/changed id — rename local link (not a cross-folder merge).
        row.id = id.clone();
    } else {
        s.projects.push(LinkedProject {
            id: id.clone(),
            path: path.clone(),
        });
    }
    s.repo = path;
    s.project = id;
    let _ = _create;
    Ok(())
}

/// Link mode (refuses cross-folder id collisions). Prefer [`link_project`] when you need the error.
pub fn upsert_linked_project(s: &mut Settings, project_id: &str, local_path: &Path) {
    let _ = link_project(s, project_id, local_path, false);
}

pub fn merge_json(base: &mut Value, patch: Value) {
    if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            b.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn create_refuses_id_collision_across_folders() {
        let mut s = Settings::default();
        link_project(&mut s, "alpha", Path::new("/tmp/a"), true).unwrap();
        let err = link_project(&mut s, "alpha", Path::new("/tmp/b"), true).unwrap_err();
        assert!(matches!(err, LinkProjectError::IdCollision { .. }));
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].path, "/tmp/a");
    }

    #[test]
    fn same_folder_relink_is_idempotent() {
        let mut s = Settings::default();
        link_project(&mut s, "alpha", Path::new("/tmp/a"), true).unwrap();
        link_project(&mut s, "alpha", Path::new("/tmp/a"), false).unwrap();
        assert_eq!(s.projects.len(), 1);
    }
}
