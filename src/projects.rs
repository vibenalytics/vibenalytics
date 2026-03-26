use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use serde_json;
use chrono::Utc;
use crate::paths::projects_path;
use crate::hash::hash_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub path: String,
    pub path_hash: String,
    pub enabled: bool,
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectEntry>,
    #[serde(default)]
    pub default_enabled: bool,
    #[serde(default)]
    pub onboarding_completed: bool,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        ProjectRegistry {
            projects: Vec::new(),
            default_enabled: false,
            onboarding_completed: false,
        }
    }
}

pub fn read_projects(dir: &Path) -> ProjectRegistry {
    let path = projects_path(dir);
    match fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => ProjectRegistry::default(),
    }
}

pub fn write_projects(dir: &Path, registry: &ProjectRegistry) -> std::io::Result<()> {
    let path = projects_path(dir);
    let data = serde_json::to_string_pretty(registry)?;
    fs::write(path, data)
}

/// Check if a project path_hash is enabled for syncing.
/// Returns: Some(true) if enabled, Some(false) if disabled, None if not registered.
#[allow(dead_code)]
pub fn is_project_enabled(dir: &Path, ph: &str) -> Option<bool> {
    let registry = read_projects(dir);
    registry.projects.iter()
        .find(|p| p.path_hash == ph)
        .map(|p| p.enabled)
}

/// Look up group_id for a project by path_hash.
pub fn get_group_id(dir: &Path, ph: &str) -> Option<String> {
    let registry = read_projects(dir);
    registry.projects.iter()
        .find(|p| p.path_hash == ph)
        .and_then(|p| p.group_id.clone())
}

/// Set or clear group_id for a project by path_hash.
pub fn set_group_id(dir: &Path, ph: &str, group_id: Option<&str>, group_name: Option<&str>) -> Result<(), String> {
    let mut registry = read_projects(dir);
    let project = registry.projects.iter_mut()
        .find(|p| p.path_hash == ph)
        .ok_or_else(|| "Project not found in registry".to_string())?;
    project.group_id = group_id.map(|s| s.to_string());
    project.group_name = group_name.map(|s| s.to_string());
    write_projects(dir, &registry).map_err(|e| format!("Failed to write projects.json: {e}"))
}

/// Find a project by name or by path_hash derived from cwd.
pub fn find_project<'a>(registry: &'a ProjectRegistry, name_or_cwd: &str) -> Option<usize> {
    // Try by name first
    if let Some(idx) = registry.projects.iter().position(|p| p.name == name_or_cwd) {
        return Some(idx);
    }
    // Try by path match
    if let Some(idx) = registry.projects.iter().position(|p| p.path == name_or_cwd) {
        return Some(idx);
    }
    // Try by path_hash of the argument (treating it as a path)
    let ph = hash_path(name_or_cwd);
    registry.projects.iter().position(|p| p.path_hash == ph)
}

/// Add a project to the registry. Always enabled (explicit user action).
/// Returns Ok(name) on success, Err(message) on failure.
pub fn add_project(dir: &Path, project_path: &str) -> Result<String, String> {
    let canonical = fs::canonicalize(project_path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| project_path.to_string());

    let ph = hash_path(&canonical);
    let name = canonical.rsplit('/').find(|s| !s.is_empty()).unwrap_or("unknown").to_string();

    let mut registry = read_projects(dir);

    if let Some(existing) = registry.projects.iter().find(|p| p.path_hash == ph) {
        if existing.enabled {
            return Err(format!("\"{}\" is already tracked (active).", existing.name));
        } else {
            return Err(format!("\"{}\" is already tracked (paused). Run: vibenalytics project enable", existing.name));
        }
    }

    registry.projects.push(ProjectEntry {
        name: name.clone(),
        path: canonical.clone(),
        path_hash: ph,
        enabled: true, // explicit add = always enabled
        added_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        group_id: None,
        group_name: None,
    });

    write_projects(dir, &registry).map_err(|e| format!("Failed to write projects.json: {e}"))?;
    Ok(name)
}

/// Remove a project by name or cwd. Returns Ok(name) or Err(message).
pub fn remove_project(dir: &Path, name_or_cwd: &str) -> Result<String, String> {
    let mut registry = read_projects(dir);
    let idx = find_project(&registry, name_or_cwd)
        .ok_or_else(|| format!("Error: Project \"{}\" not found. Run: vibenalytics project list", name_or_cwd))?;
    let name = registry.projects[idx].name.clone();
    registry.projects.remove(idx);
    write_projects(dir, &registry).map_err(|e| format!("{e}"))?;
    Ok(name)
}

/// Enable a project. Returns Ok(name) or Err(message).
pub fn enable_project(dir: &Path, name_or_cwd: &str) -> Result<String, String> {
    let mut registry = read_projects(dir);
    let idx = find_project(&registry, name_or_cwd)
        .ok_or_else(|| format!("Error: This directory is not a tracked project. Run: vibenalytics project add"))?;
    if registry.projects[idx].enabled {
        return Err(format!("\"{}\" is already active.", registry.projects[idx].name));
    }
    registry.projects[idx].enabled = true;
    let name = registry.projects[idx].name.clone();
    write_projects(dir, &registry).map_err(|e| format!("{e}"))?;
    Ok(name)
}

/// Disable a project. Returns Ok(name) or Err(message).
pub fn disable_project(dir: &Path, name_or_cwd: &str) -> Result<String, String> {
    let mut registry = read_projects(dir);
    let idx = find_project(&registry, name_or_cwd)
        .ok_or_else(|| format!("Error: This directory is not a tracked project. Run: vibenalytics project add"))?;
    if !registry.projects[idx].enabled {
        return Err(format!("\"{}\" is already paused.", registry.projects[idx].name));
    }
    registry.projects[idx].enabled = false;
    let name = registry.projects[idx].name.clone();
    write_projects(dir, &registry).map_err(|e| format!("{e}"))?;
    Ok(name)
}

/// Filter sessions by project enabled state, auto-registering new projects in auto mode.
/// Returns the filtered sessions.
pub fn filter_sessions_by_enabled<S: HasProjectHash>(dir: &Path, mut sessions: Vec<S>) -> Vec<S> {
    let mut registry = read_projects(dir);

    let enabled_hashes: std::collections::HashSet<String> = registry.projects.iter()
        .filter(|p| p.enabled)
        .map(|p| p.path_hash.clone())
        .collect();
    let disabled_hashes: std::collections::HashSet<String> = registry.projects.iter()
        .filter(|p| !p.enabled)
        .map(|p| p.path_hash.clone())
        .collect();

    let mut new_projects: Vec<(String, String, String, bool)> = Vec::new(); // (name, path_hash, path, enabled)

    sessions.retain(|s| {
        let ph = s.path_hash();
        let name = s.project_name();
        if ph.is_empty() { return true; }
        if disabled_hashes.contains(ph) { return false; }
        if enabled_hashes.contains(ph) { return true; }
        // Unregistered project — always register, enabled state follows default_enabled
        new_projects.push((name.to_string(), ph.to_string(), s.project_path().to_string(), registry.default_enabled));
        registry.default_enabled // only sync if auto mode
    });

    // Register newly discovered projects
    if !new_projects.is_empty() {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        for (name, ph, path, enabled) in &new_projects {
            if !registry.projects.iter().any(|p| p.path_hash == *ph) {
                registry.projects.push(ProjectEntry {
                    name: name.clone(),
                    path: path.clone(),
                    path_hash: ph.clone(),
                    enabled: *enabled,
                    added_at: now.clone(),
                    group_id: None,
                    group_name: None,
                });
            }
        }
        let _ = write_projects(dir, &registry);
    }

    sessions
}

/// Trait for types that carry project hash and name (used by filter_sessions_by_enabled).
pub trait HasProjectHash {
    fn path_hash(&self) -> &str;
    fn project_name(&self) -> &str;
    fn project_path(&self) -> &str { "" }
}

/// Register multiple discovered projects at once (used by onboarding).
/// Each tuple: (name, path, path_hash, enabled).
pub fn register_projects_bulk(
    dir: &Path,
    selections: &[(String, String, String, bool)],
    default_enabled: bool,
) -> Result<(), String> {
    let mut registry = read_projects(dir);
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    for (name, path, path_hash, enabled) in selections {
        if registry.projects.iter().any(|p| p.path_hash == *path_hash) {
            continue;
        }
        registry.projects.push(ProjectEntry {
            name: name.clone(),
            path: path.clone(),
            path_hash: path_hash.clone(),
            enabled: *enabled,
            added_at: now.clone(),
            group_id: None,
            group_name: None,
        });
    }

    registry.default_enabled = default_enabled;
    registry.onboarding_completed = true;
    write_projects(dir, &registry).map_err(|e| format!("Failed to write projects.json: {e}"))
}
