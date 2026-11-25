//! TOOL-036 assembly wiring order (Go `internal/assembly/boot/build.go:359-455`;
//! docs/contracts/tools.md §3.0).
//!
//! Covers the frozen sixteen-step supplier table: contiguous numbering, unique
//! ids, the capability proxy registered immediately before the catalog gate,
//! the composite step's internal Go order, and the insertion points the
//! parallel domain slices anchor to.

use reasonix_tools::assembly::{
    EXPECTED_STEP_COUNT, GATE_SUPPLIER_ID, PROXY_SUPPLIER_ID, Supplier, builtin_suppliers,
    insertion_points, supplier_at, supplier_by_id, validate,
};

#[test]
fn the_table_holds_the_frozen_sixteen_steps_in_order() {
    let suppliers = builtin_suppliers();
    assert_eq!(suppliers.len(), EXPECTED_STEP_COUNT);
    for (i, supplier) in suppliers.iter().enumerate() {
        assert_eq!(
            supplier.step as usize,
            i + 1,
            "step numbers are contiguous 1..={EXPECTED_STEP_COUNT}"
        );
    }
    validate().expect("the frozen supplier table passes its own guard");
}

#[test]
fn every_supplier_carries_a_go_symbol_note_and_non_empty_ops() {
    for supplier in builtin_suppliers() {
        assert!(!supplier.go_symbol.is_empty(), "step {}", supplier.step);
        assert!(!supplier.note.is_empty(), "step {}", supplier.step);
        assert!(!supplier.ops.is_empty(), "step {}", supplier.step);
        assert!(!supplier.id.is_empty(), "step {}", supplier.step);
    }
}

#[test]
fn the_capability_proxy_is_step_fifteen_and_the_gate_step_sixteen() {
    let proxy = supplier_by_id(PROXY_SUPPLIER_ID).expect("proxy supplier");
    let gate = supplier_by_id(GATE_SUPPLIER_ID).expect("gate supplier");
    assert_eq!(proxy.step, 15);
    assert_eq!(gate.step, 16);
    assert_eq!(
        proxy.step + 1,
        gate.step,
        "proxy is the last tool-adding step"
    );

    let proxy_at = supplier_at(15).expect("step 15");
    let gate_at = supplier_at(16).expect("step 16");
    assert_eq!(proxy_at.id, PROXY_SUPPLIER_ID);
    assert_eq!(gate_at.id, GATE_SUPPLIER_ID);
    assert!(supplier_at(17).is_none());
}

#[test]
fn step_three_preserves_the_go_chain_order() {
    let step3 = supplier_at(3).expect("step 3");
    assert_eq!(
        step3.ops,
        &[
            "addViewImage",
            "addPresentFiles",
            "bindFileViews",
            "peerTools",
            "addSettingTool",
            "addSystemOne",
            "addAdvisor",
            "addWebSearch",
            "addFetchChain",
        ]
    );
}

#[test]
fn step_twelve_leaves_the_slash_command_tool_last() {
    let step12 = supplier_at(12).expect("step 12");
    assert_eq!(step12.ops.last(), Some(&"registerSlashCommandTool"));
}

#[test]
fn supplier_ids_are_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for supplier in builtin_suppliers() {
        assert!(
            seen.insert(supplier.id),
            "duplicate supplier id {}",
            supplier.id
        );
    }
}

#[test]
fn insertion_points_anchor_to_real_suppliers() {
    let points = insertion_points();
    let domains: Vec<&str> = points.iter().map(|p| p.domain).collect();
    assert_eq!(domains, vec!["web", "browser", "computer", "media", "plan"]);
    for point in points {
        let anchor = supplier_by_id(point.anchor_id).expect("anchor supplier exists");
        assert_eq!(anchor.step, point.anchor_step);
        assert!(
            anchor.ops.contains(&point.slot),
            "slot is in the anchor step"
        );
        assert!(!point.shape.is_empty());
    }

    let browser = points.iter().find(|p| p.domain == "browser").unwrap();
    let computer = points.iter().find(|p| p.domain == "computer").unwrap();
    assert_eq!(browser.anchor_step, 5);
    assert_eq!(computer.anchor_step, 5);

    let media = points.iter().find(|p| p.domain == "media").unwrap();
    assert_eq!(media.anchor_step, 3);

    let plan = points.iter().find(|p| p.domain == "plan").unwrap();
    assert_eq!(plan.anchor_step, 9);
}

#[test]
fn validation_failures_carry_readable_messages() {
    use reasonix_tools::assembly::AssemblyError;

    let count = AssemblyError::StepCountMismatch {
        count: 15,
        expected: EXPECTED_STEP_COUNT,
    };
    assert!(count.to_string().contains("want 16"), "{count}");

    let proxy = AssemblyError::ProxyNotLast {
        proxy: 14,
        gate: 16,
    };
    assert!(proxy.to_string().contains("proxy"), "{proxy}");

    let duplicate = AssemblyError::DuplicateId("builtins");
    assert!(duplicate.to_string().contains("builtins"), "{duplicate}");
}

#[test]
fn the_go_step_symbols_track_the_contract_wiring() {
    let suppliers = builtin_suppliers();
    let by_id: std::collections::HashMap<&str, &Supplier> =
        suppliers.iter().map(|s| (s.id, s)).collect();
    assert!(by_id["registry"].go_symbol.contains("newToolRegistry"));
    assert!(by_id["builtins"].go_symbol.contains("addBuiltins"));
    assert!(by_id["mcp_machine"].go_symbol.contains("wireMCP"));
    assert!(by_id["capability_proxy"].go_symbol.contains("caps.proxy"));
    assert!(
        by_id["catalog_gate"]
            .go_symbol
            .contains("gateSkillsOnCatalog")
    );
}
