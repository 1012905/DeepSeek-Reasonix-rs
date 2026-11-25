//! TOOL-036 exercised end to end: the live registry is built by walking the
//! frozen sixteen-step order, so each domain's tools must appear at their
//! insertion point, exactly once.

use reasonix_tools::assembly;
use reasonix_tools::builtin;
use reasonix_tools::wiring::{WireEnv, assemble_registry, assembled_tools, step_of};

fn env() -> WireEnv {
    WireEnv::new("C:/work/space")
}

#[test]
fn the_walk_keeps_every_frozen_step_in_order() {
    // A walk that reorders or skips a step would still produce tools; the
    // table plus the proxy/gate guard is what pins the numbering.
    assembly::validate().expect("the frozen table still validates");
    let suppliers = assembly::builtin_suppliers();
    let mut seen: Vec<u16> = suppliers.iter().map(|s| s.step).collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        (1..=assembly::EXPECTED_STEP_COUNT as u16).collect::<Vec<u16>>(),
        "every step 1..=16 must be walked"
    );
}

#[test]
fn every_domain_lands_at_its_insertion_point() {
    let env = env();
    let anchors: [(&str, u16); 10] = [
        ("web_fetch", 2),
        ("import_web_image", 2),
        ("read_video", 3),
        ("render_chart", 3),
        ("present_files", 3),
        ("browser_open", 5),
        ("computer_read", 5),
        ("todo_write", 9),
        ("compress", 2),
        ("context_budget", 2),
    ];
    for (name, want) in anchors {
        assert_eq!(
            step_of(name, &env),
            Some(want),
            "{name} must register at step {want}"
        );
    }
}

#[test]
fn skill_and_session_tools_land_in_their_batches() {
    let env = env();
    assert_eq!(step_of("complete_step", &env), Some(9));
    assert_eq!(step_of("docs", &env), Some(9));
    assert_eq!(step_of("ask", &env), Some(9));
    assert_eq!(step_of("run_skill", &env), Some(12));
    assert_eq!(step_of("read_skill", &env), Some(12));
    assert_eq!(step_of("slash_command", &env), Some(12));
    assert_eq!(step_of("install_source", &env), Some(11));
    assert_eq!(step_of("spawn_agents", &env), Some(8));
}

#[test]
fn no_name_is_registered_twice() {
    let env = env();
    let names: Vec<String> = assembled_tools(&env)
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "a duplicate registration would shadow a tool silently"
    );
}

#[test]
fn the_runtime_registry_resolves_the_domains_and_the_new_tools() {
    let registry = assemble_registry("C:/work/space");
    for name in [
        "web_fetch",
        "browser_read",
        "computer_act",
        "read_video",
        "render_chart",
        "todo_write",
        "docs",
        "run_skill",
        "install_skill",
        "spawn_agents",
        "read_subagent_result",
        "view_image",
        "await_user",
    ] {
        assert!(
            registry.get(name).is_some(),
            "{name} must be resolvable from the assembled registry"
        );
    }
}

#[test]
fn the_conditional_surfaces_anchor_to_real_suppliers() {
    // The eight conditional faces (view image, web search, fetch chain,
    // install source, slash command, mcp connect and the two extension
    // surfaces) each name a supplier and a step; a drift in either fails here
    // instead of silently landing at a stale anchor.
    let anchors =
        assembly::resolve_conditional_anchors(builtin::conditional::conditional_surfaces())
            .expect("every conditional surface anchors to a frozen supplier");
    assert_eq!(anchors.len(), 8);
}
