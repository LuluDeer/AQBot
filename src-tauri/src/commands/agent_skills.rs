use std::collections::HashSet;
use std::path::Path;

use open_agent_sdk::skills::{sync_skill_runtime, SkillRegistry, SkillRuntimeMap};

pub struct PreparedAgentSkills {
    pub registry: SkillRegistry,
    pub summary: Option<String>,
    pub runtime: Option<SkillRuntimeMap>,
}

pub fn prepare_agent_skills(
    home: &Path,
    disabled: HashSet<String>,
    inject_summary: bool,
    workspace: Option<&Path>,
    session_id: &str,
) -> Result<PreparedAgentSkills, String> {
    let mut registry = SkillRegistry::new();
    for skill in open_agent_sdk::skills::load_all_global(home) {
        registry.register(skill);
    }
    registry.set_disabled(disabled);

    let runtime = match workspace {
        Some(workspace) if !workspace.as_os_str().is_empty() => {
            if inject_summary {
                let enabled: Vec<_> = registry.all_enabled().into_iter().cloned().collect();
                Some(sync_skill_runtime(workspace, session_id, &enabled)?)
            } else if workspace.exists() {
                Some(SkillRuntimeMap::for_session(workspace, session_id)?)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(runtime) = &runtime {
        registry.apply_mapped_dirs(runtime);
    }

    let summary = if inject_summary {
        let summary = registry.generate_context_summary();
        if summary.is_empty() {
            None
        } else {
            Some(summary)
        }
    } else {
        None
    };
    Ok(PreparedAgentSkills {
        registry,
        summary,
        runtime,
    })
}
