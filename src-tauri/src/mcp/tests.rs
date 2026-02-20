use super::server::{
    infer_plan_control_op, tool_api_path, tool_definitions, update_plan_session_affinity,
    with_plan_session_hint,
};
use serde_json::json;

#[test]
fn test_tool_api_path_known_tools() {
    assert_eq!(
        tool_api_path("aperture_context_preview"),
        Some("/context/preview")
    );
    assert_eq!(
        tool_api_path("aperture_context_read"),
        Some("/context/read")
    );
    assert_eq!(
        tool_api_path("aperture_context_search"),
        Some("/context/search")
    );
    assert_eq!(
        tool_api_path("aperture_context_plan"),
        Some("/context/plan")
    );
    assert_eq!(
        tool_api_path("aperture_context_status"),
        Some("/context/status")
    );
}

#[test]
fn test_tool_api_path_unknown() {
    assert_eq!(tool_api_path("unknown_tool"), None);
    assert_eq!(tool_api_path(""), None);
}

#[test]
fn test_tool_definitions_count() {
    let defs = tool_definitions();
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 5);
}

#[test]
fn test_tool_definitions_have_required_fields() {
    let defs = tool_definitions();
    for tool in defs.as_array().unwrap() {
        assert!(tool.get("name").is_some(), "tool missing name");
        assert!(
            tool.get("description").is_some(),
            "tool missing description"
        );
        let schema = tool.get("inputSchema").expect("tool missing inputSchema");
        assert_eq!(schema["type"], "object");
    }
}

#[test]
fn test_tool_definitions_names_match_api_paths() {
    let defs = tool_definitions();
    for tool in defs.as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool_api_path(name).is_some(),
            "tool '{name}' has no API path mapping"
        );
    }
}

#[test]
fn test_tool_definitions_include_plan_split_schema() {
    let defs = tool_definitions();
    let plan_tool = defs
        .as_array()
        .and_then(|tools| {
            tools.iter().find(|tool| {
                tool.get("name").and_then(|v| v.as_str()) == Some("aperture_context_plan")
            })
        })
        .expect("plan tool should exist");

    assert!(
        plan_tool
            .get("inputSchema")
            .and_then(|schema| schema.get("properties"))
            .and_then(|props| props.get("split"))
            .is_some(),
        "plan schema should include split instructions"
    );
}

#[test]
fn test_infer_plan_control_op_defaults() {
    assert_eq!(infer_plan_control_op(&json!({})), "preview");
    assert_eq!(infer_plan_control_op(&json!({"archive": ["b1"]})), "stage");
    assert_eq!(
        infer_plan_control_op(&json!({"control": {"op": "commit"}})),
        "commit"
    );
}

#[test]
fn test_with_plan_session_hint_includes_stage_ops() {
    let stage_payload = with_plan_session_hint(&json!({"archive": ["b1"]}), "stage", Some("s1"));
    assert_eq!(stage_payload["_aperture_session_id"], "s1");

    let commit_payload =
        with_plan_session_hint(&json!({"control": {"op": "commit"}}), "commit", Some("s1"));
    assert_eq!(commit_payload["_aperture_session_id"], "s1");
}

#[test]
fn test_update_plan_session_affinity_lifecycle() {
    let mut affinity = None;
    update_plan_session_affinity(
        &mut affinity,
        "stage",
        false,
        &json!({"session_id": "session-a"}),
    );
    assert_eq!(affinity.as_deref(), Some("session-a"));

    update_plan_session_affinity(
        &mut affinity,
        "preview",
        true,
        &json!({"session_id": "session-b"}),
    );
    assert_eq!(affinity.as_deref(), Some("session-a"));

    update_plan_session_affinity(
        &mut affinity,
        "commit",
        false,
        &json!({"session_id": "session-a"}),
    );
    assert!(affinity.is_none());
}
