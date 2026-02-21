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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectEntry>,
    #[serde(default)]
    pub default_enabled: bool,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        ProjectRegistry {
            projects: Vec::new(),
            default_enabled: false,
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

/// Add a project to the registry. Returns Ok(name) on success, Err(message) on failure.
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
        enabled: true,
        added_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
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
