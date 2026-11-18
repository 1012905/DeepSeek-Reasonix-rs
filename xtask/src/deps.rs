//! Crate dependency-direction enforcement (AGENTS.md 4.2, ADR-0001).
//!
//! "cargo xtask deps --check" walks every member manifest under "crates/"
//! and "apps/", then verifies each internal edge against the allowed graph.
//! It fails closed: a workspace package missing from the allowed map is an
//! error, so new crates must be registered here and in ADR-0001 first.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::manifest::{self, Manifest};

/// The test-only kit; exempt from direction rules and allowed as a
/// dev-dependency (or build-dependency) of any crate.
const TESTKIT: &str = "reasonix-testkit";

/// Every package registered in the ADR-0001 dependency graph. A dependency
/// on any of these names counts as internal even if the target manifest is
/// not part of the walked workspace, so removing a member cannot silently
/// downgrade its edges to "external".
pub const REGISTERED: &[&str] = &[
    "reasonix-core",
    "reasonix-contract",
    "reasonix-platform",
    "reasonix-config",
    "reasonix-safety",
    "reasonix-state",
    "reasonix-provider",
    "reasonix-tools",
    "reasonix-ext",
    "reasonix-agent",
    "reasonix-session",
    "reasonix-cli",
    "reasonix-tui",
    "reasonix-serve",
    "reasonix-acp",
    "reasonix-assembly",
    "reasonix",
    "reasonix-studio-host",
    TESTKIT,
];

/// Dependency policy for one workspace package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Only these internal crates may be depended on.
    Allowed(&'static [&'static str]),
    /// Exempt from direction rules.
    Exempt,
    /// Not registered in the ADR-0001 graph.
    Unknown,
}

/// The allowed internal-dependency graph (ADR-0001). Keep in sync with
/// docs/adr/0001-crate-layout.md; xtask fails closed on unregistered names.
pub fn policy(name: &str) -> Policy {
    match name {
        // Layer 0: nothing below.
        "reasonix-core" => Policy::Allowed(&[]),
        // Layer 1: contracts sit on core.
        "reasonix-contract" => Policy::Allowed(&["reasonix-core"]),
        // Layer 2: platform and configuration.
        "reasonix-platform" => Policy::Allowed(&["reasonix-contract", "reasonix-core"]),
        "reasonix-config" => {
            Policy::Allowed(&["reasonix-contract", "reasonix-core", "reasonix-platform"])
        }
        // Layer 3: safety and state.
        "reasonix-safety" => {
            Policy::Allowed(&["reasonix-contract", "reasonix-core", "reasonix-platform"])
        }
        "reasonix-state" => {
            Policy::Allowed(&["reasonix-contract", "reasonix-core", "reasonix-platform"])
        }
        // Layer 4: providers, tools, extensions.
        "reasonix-provider" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-platform",
            "reasonix-safety",
        ]),
        "reasonix-tools" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-platform",
            "reasonix-safety",
        ]),
        "reasonix-ext" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-platform",
            "reasonix-state",
        ]),
        // Layer 5: the agent loop.
        "reasonix-agent" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-platform",
            "reasonix-safety",
            "reasonix-state",
            "reasonix-tools",
            "reasonix-provider",
        ]),
        // Layer 6: session orchestration.
        "reasonix-session" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-platform",
            "reasonix-safety",
            "reasonix-state",
            "reasonix-tools",
            "reasonix-provider",
            "reasonix-agent",
        ]),
        // Layer 7: frontends; cli dispatches to its siblings.
        "reasonix-cli" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-config",
            "reasonix-platform",
            "reasonix-safety",
            "reasonix-state",
            "reasonix-provider",
            "reasonix-tools",
            "reasonix-ext",
            "reasonix-agent",
            "reasonix-session",
            "reasonix-tui",
            "reasonix-serve",
            "reasonix-acp",
        ]),
        "reasonix-tui" | "reasonix-serve" | "reasonix-acp" => Policy::Allowed(&[
            "reasonix-contract",
            "reasonix-core",
            "reasonix-config",
            "reasonix-platform",
            "reasonix-safety",
            "reasonix-state",
            "reasonix-provider",
            "reasonix-tools",
            "reasonix-ext",
            "reasonix-agent",
            "reasonix-session",
        ]),
        // Layer 8: composition root (AGENTS.md 4.2).
        "reasonix-assembly" => Policy::Allowed(&[
            "reasonix-cli",
            "reasonix-tui",
            "reasonix-serve",
            "reasonix-acp",
            "reasonix-session",
            "reasonix-agent",
            "reasonix-ext",
            "reasonix-platform",
        ]),
        // Layer 9: binaries go through assembly only.
        "reasonix" | "reasonix-studio-host" => Policy::Allowed(&["reasonix-assembly"]),
        // Test-only kit.
        TESTKIT => Policy::Exempt,
        _ => Policy::Unknown,
    }
}

/// Result of checking a workspace: violations and the internal edge count.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CheckResult {
    pub violations: Vec<String>,
    pub edges: usize,
}

/// Checks the internal dependency edges of every member manifest.
pub fn check(manifests: &[(PathBuf, Manifest)]) -> CheckResult {
    let member_names: HashSet<&str> = manifests.iter().map(|(_, m)| m.name.as_str()).collect();
    let mut result = CheckResult::default();
    for (path, manifest) in manifests {
        match policy(&manifest.name) {
            Policy::Exempt => {}
            Policy::Unknown => result.violations.push(format!(
                "{}: package {} is not in the ADR-0001 dependency graph; \
                 register it in xtask/src/deps.rs and docs/adr/0001-crate-layout.md",
                path.display(),
                manifest.name
            )),
            Policy::Allowed(allowed) => {
                for (section, deps) in &manifest.deps {
                    for dep in deps {
                        if !member_names.contains(dep.as_str())
                            && !REGISTERED.contains(&dep.as_str())
                        {
                            continue; // external crate: not our graph
                        }
                        if dep == TESTKIT {
                            if section == "dev-dependencies" || section == "build-dependencies" {
                                continue; // test-only kit, always allowed here
                            }
                            result.violations.push(format!(
                                "{}: {} may depend on {} only as a dev-dependency (found in {})",
                                path.display(),
                                manifest.name,
                                TESTKIT,
                                section
                            ));
                            continue;
                        }
                        result.edges += 1;
                        if !allowed.contains(&dep.as_str()) {
                            result.violations.push(format!(
                                "{}: {} may not depend on {} (section {}); allowed: {}",
                                path.display(),
                                manifest.name,
                                dep,
                                section,
                                allowed.join(", ")
                            ));
                        }
                    }
                }
            }
        }
    }
    result
}

/// Runs the dependency-direction gate over the repository workspace.
pub fn run(args: &[String]) -> ExitCode {
    match args {
        [] => {}
        [flag] if flag == "--check" => {}
        other => {
            eprintln!("xtask deps: unknown arguments: {}", other.join(" "));
            return ExitCode::from(2);
        }
    }
    let root = manifest::repo_root();
    let manifests = match manifest::workspace_manifests(&root) {
        Ok(manifests) => manifests,
        Err(err) => {
            eprintln!("xtask deps: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = check(&manifests);
    if result.violations.is_empty() {
        println!(
            "xtask deps: OK ({} manifests, {} internal edges)",
            manifests.len(),
            result.edges
        );
        ExitCode::SUCCESS
    } else {
        for violation in &result.violations {
            eprintln!("xtask deps: {violation}");
        }
        eprintln!(
            "xtask deps: FAILED ({} violation(s) over {} manifests)",
            result.violations.len(),
            manifests.len()
        );
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str, deps: &[(&str, Vec<&str>)]) -> (PathBuf, Manifest) {
        (
            PathBuf::from(format!("Cargo-{name}.toml")),
            Manifest {
                name: name.to_owned(),
                deps: deps
                    .iter()
                    .map(|(section, names)| {
                        (
                            (*section).to_owned(),
                            names.iter().map(|n| (*n).to_owned()).collect(),
                        )
                    })
                    .collect(),
            },
        )
    }

    #[test]
    fn core_depends_on_nothing() {
        let result = check(&[manifest("reasonix-core", &[])]);
        assert_eq!(result, CheckResult::default());
    }

    #[test]
    fn allowed_edges_pass() {
        let manifests = vec![
            manifest("reasonix-core", &[]),
            manifest(
                "reasonix-contract",
                &[("dependencies", vec!["reasonix-core"])],
            ),
        ];
        let result = check(&manifests);
        assert!(result.violations.is_empty());
        assert_eq!(result.edges, 1);
    }

    #[test]
    fn disallowed_edges_fail() {
        let manifests = vec![
            manifest("reasonix-core", &[]),
            manifest(
                "reasonix-contract",
                &[("dependencies", vec!["reasonix-agent"])],
            ),
        ];
        let result = check(&manifests);
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].contains("may not depend on reasonix-agent"));
    }

    #[test]
    fn cli_may_use_sibling_frontends() {
        let result = check(&[manifest(
            "reasonix-cli",
            &[(
                "dependencies",
                vec!["reasonix-tui", "reasonix-serve", "reasonix-acp"],
            )],
        )]);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn frontends_may_not_use_cli() {
        let result = check(&[manifest(
            "reasonix-tui",
            &[("dependencies", vec!["reasonix-cli"])],
        )]);
        assert_eq!(result.violations.len(), 1);
    }

    #[test]
    fn assembly_may_not_reach_config() {
        let result = check(&[manifest(
            "reasonix-assembly",
            &[("dependencies", vec!["reasonix-config"])],
        )]);
        assert_eq!(result.violations.len(), 1);
    }

    #[test]
    fn binaries_go_through_assembly_only() {
        let ok = check(&[manifest(
            "reasonix",
            &[("dependencies", vec!["reasonix-assembly"])],
        )]);
        assert!(ok.violations.is_empty());
        let bad = check(&[manifest(
            "reasonix",
            &[("dependencies", vec!["reasonix-cli"])],
        )]);
        assert_eq!(bad.violations.len(), 1);
    }

    #[test]
    fn testkit_is_dev_dep_only() {
        let ok = check(&[manifest(
            "reasonix-state",
            &[("dev-dependencies", vec!["reasonix-testkit"])],
        )]);
        assert!(ok.violations.is_empty());
        let bad = check(&[manifest(
            "reasonix-state",
            &[("dependencies", vec!["reasonix-testkit"])],
        )]);
        assert_eq!(bad.violations.len(), 1);
    }

    #[test]
    fn unknown_packages_fail_closed() {
        let result = check(&[manifest("reasonix-mystery", &[])]);
        assert_eq!(result.violations.len(), 1);
        assert!(result.violations[0].contains("not in the ADR-0001 dependency graph"));
    }

    #[test]
    fn external_dependencies_are_ignored() {
        let result = check(&[manifest(
            "reasonix-core",
            &[("dependencies", vec!["serde"])],
        )]);
        assert!(result.violations.is_empty());
        assert_eq!(result.edges, 0);
    }

    #[test]
    fn no_package_depends_on_itself() {
        for name in REGISTERED {
            if let Policy::Allowed(allowed) = policy(name) {
                assert!(!allowed.contains(name), "{name} depends on itself");
            }
        }
    }
}
