use std::path::Path;
use serde_json::Value;
use crate::config::{config_get, DEFAULT_API_BASE};
use crate::hash::hash_path;
use crate::http::{http_get, http_post, http_delete};
use crate::projects::{read_projects, set_group_id, find_project};

fn require_api_key(dir: &Path) -> Result<String, String> {
    config_get(dir, "apiKey").ok_or_else(|| "Not logged in. Run: vibenalytics login".to_string())
}

fn current_project_info() -> Result<(String, String, String), String> {
    let cwd = std::env::current_dir()
        .map_err(|_| "Cannot determine current directory".to_string())?;
    let canonical = std::fs::canonicalize(&cwd)
        .unwrap_or(cwd.clone())
        .to_string_lossy()
        .to_string();
    let path_hash = hash_path(&canonical);
    let name = canonical.rsplit('/').find(|s| !s.is_empty()).unwrap_or("unknown").to_string();
    Ok((canonical, path_hash, name))
}

fn parse_api_response(body: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("Invalid response: {e}"))?;
    if v.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(v)
    } else {
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
        Err(err.to_string())
    }
}

/// `vibenalytics group connect <identifier>`
/// Resolves the group by identifier, links the current project, and persists group_id locally.
pub fn cmd_connect(dir: &Path, identifier: &str) -> i32 {
    let api_key = match require_api_key(dir) {
        Ok(k) => k,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let (canonical, path_hash, project_name) = match current_project_info() {
        Ok(v) => v,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    // Check project is registered
    let registry = read_projects(dir);
    if find_project(&registry, &canonical).is_none() {
        eprintln!("This directory is not a tracked project.");
        eprintln!("Run: vibenalytics project add");
        return 1;
    }

    let api_base = DEFAULT_API_BASE;

    // Step 1: Resolve group by identifier
    let resolve_url = format!(
        "{api_base}/project-groups/resolve?identifier={}",
        urlencodestr(identifier)
    );
    let (status, body) = match http_get(&resolve_url, Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    if status != 200 {
        let v: Value = serde_json::from_str(&body).unwrap_or_default();
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Group not found");
        eprintln!("Error: {err}");
        if status == 404 {
            eprintln!("\nMake sure the group identifier is correct.");
            eprintln!("Ask your team admin for the group identifier, or check the web dashboard.");
        }
        return 1;
    }

    let resp = match parse_api_response(&body) {
        Ok(v) => v,
        Err(e) => { eprintln!("Error: {e}"); return 1; }
    };

    let data = &resp["data"];
    let group_id = data["groupId"].as_str().unwrap_or("");
    let group_name = data["name"].as_str().unwrap_or(identifier);
    let team_name = data["teamName"].as_str();

    if group_id.is_empty() {
        eprintln!("Error: Server returned empty group ID");
        return 1;
    }

    // Step 2: Link project to group
    let link_url = format!("{api_base}/project-groups/{group_id}/link");
    let link_body = serde_json::json!({
        "pathHash": path_hash,
        "projectName": project_name
    });
    let (link_status, link_resp) = match http_post(&link_url, &link_body.to_string(), Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Link request failed: {e}"); return 1; }
    };

    if link_status != 200 {
        let v: Value = serde_json::from_str(&link_resp).unwrap_or_default();
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Failed to link project");
        eprintln!("Error: {err}");
        return 1;
    }

    // Step 3: Persist group_id in local registry
    if let Err(e) = set_group_id(dir, &path_hash, Some(group_id), Some(group_name)) {
        eprintln!("Warning: Linked on server but failed to save locally: {e}");
        return 1;
    }

    if let Some(tn) = team_name {
        println!("Connected to \"{}\" (team: {})", group_name, tn);
    } else {
        println!("Connected to \"{}\"", group_name);
    }
    println!("Future syncs from this project will include the group link.");
    0
}

/// `vibenalytics group disconnect`
/// Unlinks the current project from its group.
pub fn cmd_disconnect(dir: &Path) -> i32 {
    let api_key = match require_api_key(dir) {
        Ok(k) => k,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let (canonical, path_hash, _) = match current_project_info() {
        Ok(v) => v,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let registry = read_projects(dir);
    let idx = match find_project(&registry, &canonical) {
        Some(i) => i,
        None => {
            eprintln!("This directory is not a tracked project.");
            return 1;
        }
    };

    let project = &registry.projects[idx];
    let group_id = match &project.group_id {
        Some(gid) => gid.clone(),
        None => {
            eprintln!("This project is not connected to any group.");
            return 0;
        }
    };

    let group_name = project.group_name.clone().unwrap_or_else(|| "unknown".to_string());

    let api_base = DEFAULT_API_BASE;
    let url = format!(
        "{api_base}/project-groups/{group_id}/link?pathHash={}",
        urlencodestr(&path_hash)
    );

    let (status, body) = match http_delete(&url, Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    if status != 200 {
        let v: Value = serde_json::from_str(&body).unwrap_or_default();
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Failed to unlink");
        eprintln!("Error: {err}");
        return 1;
    }

    if let Err(e) = set_group_id(dir, &path_hash, None, None) {
        eprintln!("Warning: Unlinked on server but failed to update locally: {e}");
        return 1;
    }

    println!("Disconnected from \"{}\"", group_name);
    0
}

/// `vibenalytics group status`
/// Shows the current project's group connection.
pub fn cmd_group_status(dir: &Path) -> i32 {
    let (canonical, _, _) = match current_project_info() {
        Ok(v) => v,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let registry = read_projects(dir);
    let idx = match find_project(&registry, &canonical) {
        Some(i) => i,
        None => {
            eprintln!("This directory is not a tracked project.");
            eprintln!("Run: vibenalytics project add");
            return 1;
        }
    };

    let project = &registry.projects[idx];
    println!("Project: {}", project.name);
    println!("Status:  {}", if project.enabled { "active" } else { "paused" });

    match (&project.group_id, &project.group_name) {
        (Some(_gid), Some(name)) => {
            println!("Group:   {}", name);
        }
        (Some(gid), None) => {
            println!("Group:   {}", gid);
        }
        _ => {
            println!("Group:   (not connected)");
        }
    }
    0
}

/// `vibenalytics group list`
/// Lists all groups the user has access to (personal + team).
pub fn cmd_group_list(dir: &Path, json_output: bool) -> i32 {
    let api_key = match require_api_key(dir) {
        Ok(k) => k,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let api_base = DEFAULT_API_BASE;

    // Fetch personal groups
    let (status, body) = match http_get(&format!("{api_base}/project-groups"), Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    let personal_groups: Vec<Value> = if status == 200 {
        let resp: Value = serde_json::from_str(&body).unwrap_or_default();
        resp["data"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };

    // Fetch teams and their groups
    let (t_status, t_body) = match http_get(&format!("{api_base}/teams"), Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    let teams: Vec<Value> = if t_status == 200 {
        let resp: Value = serde_json::from_str(&t_body).unwrap_or_default();
        resp["data"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };

    struct TeamGroup {
        team_name: String,
        groups: Vec<Value>,
    }

    let mut team_groups: Vec<TeamGroup> = Vec::new();
    for team in &teams {
        let team_id = team["id"].as_str().unwrap_or("");
        let team_name = team["name"].as_str().unwrap_or("Unknown");
        let (g_status, g_body) = match http_get(
            &format!("{api_base}/teams/{team_id}/groups"),
            Some(&api_key),
        ) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if g_status == 200 {
            let resp: Value = serde_json::from_str(&g_body).unwrap_or_default();
            let groups = resp["data"].as_array().cloned().unwrap_or_default();
            if !groups.is_empty() {
                team_groups.push(TeamGroup {
                    team_name: team_name.to_string(),
                    groups,
                });
            }
        }
    }

    if json_output {
        let output = serde_json::json!({
            "personal": personal_groups,
            "teams": team_groups.iter().map(|tg| serde_json::json!({
                "teamName": tg.team_name,
                "groups": tg.groups,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        return 0;
    }

    // Display personal groups
    if !personal_groups.is_empty() {
        println!("Personal Groups:");
        for g in &personal_groups {
            let name = g["name"].as_str().unwrap_or("?");
            let count = g["memberCount"].as_u64().unwrap_or(0);
            let id = g["id"].as_str().unwrap_or("");
            if id == "personal" {
                println!("  {:<24} {} projects (ungrouped)", name, count);
            } else {
                println!("  {:<24} {} projects", name, count);
            }
        }
    }

    // Display team groups
    for tg in &team_groups {
        println!("\nTeam: {}", tg.team_name);
        for g in &tg.groups {
            let name = g["name"].as_str().unwrap_or("?");
            let identifier = g["identifier"].as_str().unwrap_or("");
            let count = g["memberCount"].as_u64().unwrap_or(0);
            let contributors = g["contributorCount"].as_u64().unwrap_or(0);
            println!(
                "  {:<24} {} projects, {} contributors  (id: {})",
                name, count, contributors, identifier
            );
        }
    }

    if personal_groups.is_empty() && team_groups.is_empty() {
        println!("No groups found. Create one in the web dashboard.");
    }

    0
}

/// `vibenalytics team list`
pub fn cmd_team_list(dir: &Path, json_output: bool) -> i32 {
    let api_key = match require_api_key(dir) {
        Ok(k) => k,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let api_base = DEFAULT_API_BASE;
    let (status, body) = match http_get(&format!("{api_base}/teams"), Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    if status != 200 {
        let v: Value = serde_json::from_str(&body).unwrap_or_default();
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Failed to fetch teams");
        eprintln!("Error: {err}");
        return 1;
    }

    let resp: Value = serde_json::from_str(&body).unwrap_or_default();
    let teams = resp["data"].as_array().cloned().unwrap_or_default();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&teams).unwrap_or_default());
        return 0;
    }

    if teams.is_empty() {
        println!("You are not a member of any teams.");
        println!("Ask a team admin for an invite link, then run:");
        println!("  vibenalytics team join <invite-token>");
        return 0;
    }

    println!("  {:<24} {:<12} {}", "NAME", "ROLE", "MEMBERS");
    for t in &teams {
        let name = t["name"].as_str().unwrap_or("?");
        let role = t["role"].as_str().unwrap_or("?");
        let members = t["memberCount"].as_u64().unwrap_or(0);
        println!("  {:<24} {:<12} {}", name, role, members);
    }
    0
}

/// `vibenalytics team join <token>`
pub fn cmd_team_join(dir: &Path, token: &str) -> i32 {
    let api_key = match require_api_key(dir) {
        Ok(k) => k,
        Err(e) => { eprintln!("{e}"); return 1; }
    };

    let api_base = DEFAULT_API_BASE;
    let url = format!("{api_base}/teams/join/{}", urlencodestr(token));
    let (status, body) = match http_post(&url, "{}", Some(&api_key)) {
        Ok(r) => r,
        Err(e) => { eprintln!("Request failed: {e}"); return 1; }
    };

    if status != 200 {
        let v: Value = serde_json::from_str(&body).unwrap_or_default();
        let err = v.get("error").and_then(|v| v.as_str()).unwrap_or("Failed to join team");
        eprintln!("Error: {err}");
        return 1;
    }

    let resp: Value = serde_json::from_str(&body).unwrap_or_default();
    let team_name = resp["data"]["teamName"].as_str().unwrap_or("the team");
    println!("Joined \"{}\"!", team_name);
    println!("\nTo connect a project to a team group:");
    println!("  cd /path/to/your/project");
    println!("  vibenalytics group connect <group-identifier>");
    0
}

fn urlencodestr(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}
