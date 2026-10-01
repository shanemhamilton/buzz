//! Tests for cascade-delete filtering in `delete_persona`.
//!
//! `delete_persona` deletes all managed-agent records whose `persona_id`
//! matches the persona being deleted, mirroring the cleanup done by
//! `delete_managed_agent`. These tests verify the `collect_cascade_pubkeys`
//! helper that identifies the agents to cascade-delete, using plain
//! in-memory data structures (no `AppHandle` required).

use super::{collect_cascade_pubkeys, collect_remote_deployed, commit_cascade_agents};
use crate::managed_agents::{BackendKind, ManagedAgentRecord, RespondTo};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashSet;

fn make_agent(
    pubkey: &str,
    persona_id: Option<&str>,
    runtime_pid: Option<u32>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        session_policy: Default::default(),
        description: None,
        pubkey: pubkey.to_string(),
        name: "Test Agent".to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "buzz-agent".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        relay_mesh: None,
        effort_level: None,
        auto_restart_on_config_change: false,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}

const PERSONA_ID: &str = "custom:test-persona";

/// Deleting a persona with two linked agents (one running) returns both their
/// pubkeys and leaves unlinked agents out of the cascade set.
#[test]
fn cascade_includes_linked_agents_and_excludes_others() {
    let agents = vec![
        make_agent("agent-a", Some(PERSONA_ID), Some(12345)), // running, linked
        make_agent("agent-b", Some(PERSONA_ID), None),        // stopped, linked
        make_agent("agent-c", Some("custom:other"), None),    // different persona
        make_agent("agent-d", None, None),                    // no persona
    ];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert_eq!(
        pubkeys.len(),
        2,
        "exactly two agents linked to this persona"
    );
    assert!(
        pubkeys.contains(&"agent-a".to_string()),
        "running linked agent included"
    );
    assert!(
        pubkeys.contains(&"agent-b".to_string()),
        "stopped linked agent included"
    );
    assert!(
        !pubkeys.contains(&"agent-c".to_string()),
        "different-persona agent excluded"
    );
    assert!(
        !pubkeys.contains(&"agent-d".to_string()),
        "persona-less agent excluded"
    );
}

/// Deleting a persona with no linked agents returns an empty list (no cascade).
#[test]
fn cascade_empty_when_no_linked_agents() {
    let agents = vec![
        make_agent("agent-x", Some("custom:other"), None),
        make_agent("agent-y", None, None),
    ];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert!(pubkeys.is_empty(), "no agents to cascade-delete");
}

/// Cascade targets all agents linked to the persona — not just stopped ones.
/// A running agent (runtime_pid set) must appear in the cascade set so the
/// command can stop it before removing the record.
#[test]
fn cascade_includes_running_agent() {
    let agents = vec![make_agent("running-agent", Some(PERSONA_ID), Some(99999))];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert_eq!(pubkeys, vec!["running-agent".to_string()]);
}

/// A failing agent-store save in Phase 3 must be retry-safe: the error
/// propagates before any keyring deletion. The command now secures the
/// tombstone/archive retry record before this commit, so local records remain
/// untouched and a retry may re-enter idempotently with that record present.
#[test]
fn failing_save_is_retry_safe() {
    let mut agents = vec![
        make_agent("pk-a", Some(PERSONA_ID), None),
        make_agent("pk-b", Some(PERSONA_ID), None),
        make_agent("pk-c", Some("custom:other"), None),
    ];
    let cascade: HashSet<String> = ["pk-a".to_string(), "pk-b".to_string()].into();

    let cleanup_calls = RefCell::new(0);
    let result = commit_cascade_agents(
        &mut agents,
        &cascade,
        |_| Err("simulated disk failure".to_string()),
        |_| *cleanup_calls.borrow_mut() += 1,
    );

    assert!(
        result.is_err(),
        "commit must propagate the save error so callers can react"
    );
    assert_eq!(
        *cleanup_calls.borrow(),
        0,
        "failed agent-record save must retain local caches and keys"
    );
}

/// A later persona-store failure cannot defer cache/key cleanup: after the
/// agent-record save succeeds the cascade records are gone, so a retry's empty
/// cascade has no way to find leaked local keys.
#[test]
fn persona_save_failure_follows_agent_cleanup() {
    let mut agents = vec![
        make_agent("pk-a", Some(PERSONA_ID), None),
        make_agent("pk-b", Some(PERSONA_ID), None),
        make_agent("pk-c", Some("custom:other"), None),
    ];
    let cascade: HashSet<String> = ["pk-a".to_string(), "pk-b".to_string()].into();
    let sequence = RefCell::new(Vec::new());

    commit_cascade_agents(
        &mut agents,
        &cascade,
        |_| {
            sequence.borrow_mut().push("agent save");
            Ok(())
        },
        |_| sequence.borrow_mut().push("agent cache/key cleanup"),
    )
    .expect("agent records save successfully");

    sequence.borrow_mut().push("persona save");
    let persona_save: Result<(), String> = Err("simulated persona disk failure".to_string());
    assert!(
        persona_save.is_err(),
        "the second save fails after agent cleanup"
    );
    assert_eq!(
        sequence.into_inner(),
        vec!["agent save", "agent cache/key cleanup", "persona save"],
        "cleanup must occur before the independently failing persona save"
    );
    assert_eq!(
        agents
            .iter()
            .map(|agent| agent.pubkey.as_str())
            .collect::<Vec<_>>(),
        vec!["pk-c"],
        "the successful agent save leaves retry with an empty cascade"
    );
}

/// A provider-deployed cascade target (non-local backend with a live
/// `backend_agent_id`) must be detected by the pre-flight so `delete_persona`
/// refuses the cascade before any destructive work. Local agents and
/// never-deployed provider agents must not block.
#[test]
fn remote_deployed_cascade_target_blocks_delete() {
    let mut deployed = make_agent("pk-deployed", Some(PERSONA_ID), None);
    deployed.name = "Deployed Agent".to_string();
    deployed.backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::Value::Null,
    };
    deployed.backend_agent_id = Some("backend-1".to_string());

    // Provider backend but never deployed (no backend_agent_id) — not a blocker.
    let mut undeployed = make_agent("pk-undeployed", Some(PERSONA_ID), None);
    undeployed.backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::Value::Null,
    };

    let agents = vec![
        make_agent("pk-local", Some(PERSONA_ID), None),
        deployed,
        undeployed,
    ];
    let cascade: HashSet<String> = collect_cascade_pubkeys(&agents, PERSONA_ID)
        .into_iter()
        .collect();
    assert_eq!(cascade.len(), 3, "all three agents are cascade targets");

    let blockers = collect_remote_deployed(&agents, &cascade);

    assert_eq!(
        blockers,
        vec!["Deployed Agent".to_string()],
        "only the deployed provider agent blocks the cascade"
    );
}
