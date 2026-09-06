use open_agent_sdk::skills::SkillRuntimeMap;
use open_agent_sdk::PermissionDecision;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Validate that a path resolves to within the cwd.
/// For existing paths, uses canonicalize (resolves symlinks).
/// For non-existing paths, validates the parent directory.
pub fn validate_path_within_cwd(path: &str, cwd: &str) -> Result<PathBuf, String> {
    let target = Path::new(path);
    let cwd_path = Path::new(cwd);

    // Canonicalize cwd (must exist)
    let cwd_canonical = cwd_path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve working directory '{}': {}", cwd, e))?;

    // Try to resolve the target path
    let resolved = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd_path.join(target)
    };

    // If path exists, canonicalize it (resolves symlinks)
    if resolved.exists() {
        let canonical = resolved
            .canonicalize()
            .map_err(|e| format!("Cannot resolve path '{}': {}", path, e))?;
        if canonical.starts_with(&cwd_canonical) {
            Ok(canonical)
        } else {
            Err(format!(
                "Path '{}' resolves to '{}' which is outside working directory '{}'",
                path,
                canonical.display(),
                cwd_canonical.display()
            ))
        }
    } else {
        // For non-existing paths, check parent directory
        if let Some(parent) = resolved.parent() {
            if parent.exists() {
                let parent_canonical = parent
                    .canonicalize()
                    .map_err(|e| format!("Cannot resolve parent of '{}': {}", path, e))?;
                if parent_canonical.starts_with(&cwd_canonical) {
                    Ok(resolved)
                } else {
                    Err(format!(
                        "Parent of '{}' resolves outside working directory '{}'",
                        path,
                        cwd_canonical.display()
                    ))
                }
            } else {
                // Parent doesn't exist — walk up until we find an existing ancestor
                let mut ancestor = parent.to_path_buf();
                loop {
                    if ancestor.exists() {
                        let anc_canonical = ancestor
                            .canonicalize()
                            .map_err(|e| format!("Cannot resolve ancestor: {}", e))?;
                        if anc_canonical.starts_with(&cwd_canonical) {
                            return Ok(resolved);
                        } else {
                            return Err(format!(
                                "Path '{}' is outside working directory '{}'",
                                path,
                                cwd_canonical.display()
                            ));
                        }
                    }
                    if !ancestor.pop() {
                        return Err(format!(
                            "Cannot verify path '{}' against working directory",
                            path
                        ));
                    }
                }
            }
        } else {
            Err(format!("Invalid path: '{}'", path))
        }
    }
}

/// Check if a tool's path arguments are safe (within cwd).
/// Returns Some(PermissionDecision::Deny(reason)) if unsafe, None if safe or not applicable.
pub fn check_path_safety(tool_name: &str, input: &Value, cwd: &str) -> Option<PermissionDecision> {
    check_path_safety_with_runtime(tool_name, input, cwd, None, true)
}

pub fn check_path_safety_with_runtime(
    tool_name: &str,
    input: &Value,
    cwd: &str,
    runtime: Option<&SkillRuntimeMap>,
    enforce_cwd: bool,
) -> Option<PermissionDecision> {
    let name_lower = tool_name.to_lowercase();
    if matches!(
        name_lower.as_str(),
        "bash" | "shell" | "run_command" | "execute"
    ) {
        return None;
    }

    let write = is_mutating_tool(&name_lower);
    let paths = collect_tool_paths(&name_lower, input);
    if paths.is_empty() {
        return None;
    }

    for path in paths {
        if let Some(runtime) = runtime {
            match runtime.authorize(&path, cwd, write) {
                Some(Err(reason)) => return Some(PermissionDecision::Deny(reason)),
                Some(Ok(())) => continue,
                None => {}
            }
        }
        if enforce_cwd {
            if let Err(reason) = validate_path_within_cwd(&path, cwd) {
                return Some(PermissionDecision::Deny(reason));
            }
        }
    }
    None
}

fn is_mutating_tool(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "write"
            | "write_file"
            | "edit"
            | "edit_file"
            | "create"
            | "create_file"
            | "delete"
            | "delete_file"
            | "rename"
            | "move"
            | "mkdir"
            | "remove"
            | "patch"
    ) || name_lower.contains("write")
        || name_lower.contains("edit")
        || name_lower.contains("delete")
        || name_lower.contains("patch")
}

fn collect_tool_paths(name_lower: &str, input: &Value) -> Vec<String> {
    match name_lower {
        "read" | "read_file" | "write" | "write_file" | "edit" | "edit_file" | "create"
        | "create_file" | "delete" | "delete_file" | "rename" | "list_dir" | "listdir" => {
            let mut paths = Vec::new();
            if let Some(path) = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .or_else(|| input.get("file"))
                .and_then(|value| value.as_str())
            {
                paths.push(path.to_string());
            }
            if let Some(path) = input.get("new_path").and_then(|value| value.as_str()) {
                paths.push(path.to_string());
            }
            paths
        }
        "glob" | "glob_search" => {
            let mut paths = Vec::new();
            if let Some(path) = input.get("path").and_then(|value| value.as_str()) {
                paths.push(path.to_string());
            }
            if let Some(pattern) = input
                .get("pattern")
                .or_else(|| input.get("glob"))
                .and_then(|value| value.as_str())
            {
                let base_dir = extract_glob_base(pattern);
                if !base_dir.is_empty() {
                    paths.push(base_dir);
                }
            }
            paths
        }
        "grep" | "search" | "ripgrep" => input
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract the non-wildcard prefix from a glob pattern as a base directory.
/// e.g., "src/components/**/*.tsx" → "src/components"
///       "**/*.rs" → ""
///       "/absolute/path/to/*.txt" → "/absolute/path/to"
fn extract_glob_base(pattern: &str) -> String {
    let mut parts = Vec::new();
    for segment in pattern.split('/') {
        if segment.contains('*')
            || segment.contains('?')
            || segment.contains('[')
            || segment.contains('{')
        {
            break;
        }
        parts.push(segment);
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use open_agent_sdk::skills::{load_skill_from_dir, sync_skill_runtime};
    use open_agent_sdk::types::SkillSource;
    use serde_json::json;
    use std::fs;

    fn mapped_runtime() -> (tempfile::TempDir, SkillRuntimeMap, String) {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let source = root.path().join("skill");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: test\n---\n\n# Demo\n",
        )
        .unwrap();
        let skill = load_skill_from_dir(&source, SkillSource::AQBot).unwrap();
        let map = sync_skill_runtime(&workspace, "11111111-1111-4111-8111-111111111111", &[skill])
            .unwrap();
        let cwd = workspace.to_string_lossy().into_owned();
        (root, map, cwd)
    }

    #[test]
    fn read_mapped_skill_is_allowed() {
        let (_root, map, cwd) = mapped_runtime();
        let path = map.mapped_dir_for("demo").unwrap().join("SKILL.md");
        let decision = check_path_safety_with_runtime(
            "Read",
            &json!({"file_path": path.to_str().unwrap()}),
            &cwd,
            Some(&map),
            true,
        );
        assert!(decision.is_none());
    }

    #[test]
    fn write_mapped_skill_is_denied_without_cwd_enforcement() {
        let (_root, map, cwd) = mapped_runtime();
        let path = map.mapped_dir_for("demo").unwrap().join("SKILL.md");
        let decision = check_path_safety_with_runtime(
            "Write",
            &json!({"file_path": path.to_str().unwrap()}),
            &cwd,
            Some(&map),
            false,
        );
        assert!(matches!(decision, Some(PermissionDecision::Deny(_))));
    }

    #[test]
    fn outside_workspace_read_is_denied() {
        let decision = check_path_safety(
            "Read",
            &json!({"file_path": "/etc/passwd"}),
            std::env::temp_dir().to_str().unwrap(),
        );
        assert!(matches!(decision, Some(PermissionDecision::Deny(_))));
    }
}
