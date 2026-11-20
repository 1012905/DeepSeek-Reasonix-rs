//! A tiny fail-closed TOML reader for the subset of Cargo.toml this repo
//! writes, plus the workspace manifest walk.
//!
//! No external dependencies: xtask must stay cheap to build and free of
//! supply-chain surface. Supported syntax (anything else inside the parsed
//! scopes is an error, never a silent pass):
//!   - "[table]" and "[table.sub]" headers
//!   - key = "value" strings (used for "[package] name")
//!   - key = { ... } single-line inline tables and dotted "key.workspace"
//!     keys under dependency sections (the key is the datum)
//!   - "[dependencies.foo]" sub-table headers
//!   - "[target.'cfg(...)'.dependencies]" headers, mapped to their
//!     dependency section
//!
//! Multi-line inline tables are rejected: this parser reads only
//! machine-written manifests, and failing loudly beats missing an edge.

use std::fs;
use std::path::{Path, PathBuf};

/// Dependency sections of a member manifest.
pub const DEP_SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// A parsed member manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// "[package] name".
    pub name: String,
    /// Dependency names grouped by section, in file order.
    pub deps: Vec<(String, Vec<String>)>,
}

fn is_dep_section(section: &str) -> bool {
    DEP_SECTIONS.contains(&section)
}

/// Maps a "[target.'cfg(...)'.dependencies]" header to its dependency
/// section; None for anything else.
fn target_dep_section(table: &str) -> Option<String> {
    let rest = table.strip_prefix("target.")?;
    DEP_SECTIONS
        .iter()
        .find(|kind| rest.ends_with(&format!(".{kind}")))
        .map(|kind| (*kind).to_owned())
}

/// Validates a bare or quoted TOML key and returns the dependency name.
fn dep_key(key: &str, file: &Path, line_no: usize) -> Result<String, String> {
    let key = key.trim();
    let key = if let Some(inner) = key.strip_prefix('"') {
        inner
            .strip_suffix('"')
            .ok_or_else(|| format!("{}:{line_no}: unterminated quoted key", file.display()))?
    } else {
        key
    };
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(key.to_owned())
    } else {
        Err(format!(
            "{}:{line_no}: invalid dependency key {key:?}",
            file.display()
        ))
    }
}

fn push_dep(deps: &mut Vec<(String, Vec<String>)>, section: &str, name: String) {
    if let Some((_, list)) = deps.iter_mut().find(|(s, _)| s == section) {
        if !list.contains(&name) {
            list.push(name);
        }
    } else {
        deps.push((section.to_owned(), vec![name]));
    }
}

/// Parses one Cargo.toml. Fails closed: malformed structure inside dependency
/// sections or "[package]" is an error, never a silent pass.
pub fn parse_manifest(src: &str, file: &Path) -> Result<Manifest, String> {
    let mut name: Option<String> = None;
    let mut deps: Vec<(String, Vec<String>)> = Vec::new();
    let mut section = String::new();

    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix("[[") {
            // Array-of-tables headers ("[[bin]]", "[[test]]"): record the
            // section so member keys are not mistaken for dependency keys.
            let Some(table) = header.strip_suffix("]]") else {
                return Err(format!(
                    "{}:{line_no}: unterminated array-of-tables header",
                    file.display()
                ));
            };
            section = table.to_owned();
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let Some(table) = header.strip_suffix(']') else {
                return Err(format!(
                    "{}:{line_no}: unterminated table header",
                    file.display()
                ));
            };
            if let Some(mapped) = target_dep_section(table) {
                // "[target.'cfg(unix)'.dependencies]": keys are dependency
                // names in the mapped section.
                section = mapped;
            } else {
                // "[dependencies.foo]": "foo" is itself a dependency key;
                // body lines belong to foo's spec and are ignored.
                if let Some((parent, sub)) = table.split_once('.')
                    && is_dep_section(parent)
                {
                    let key = dep_key(sub, file, line_no)?;
                    push_dep(&mut deps, parent, key);
                }
                section = table.to_owned();
            }
            continue;
        }
        let Some(eq) = line.find('=') else {
            return Err(format!(
                "{}:{line_no}: expected \"key = value\", got {line:?} in [{section}]",
                file.display()
            ));
        };
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();
        if value.starts_with('{') && !value.ends_with('}') {
            return Err(format!(
                "{}:{line_no}: multi-line inline tables are not supported",
                file.display()
            ));
        }
        match section.as_str() {
            "package" => {
                if key == "name" {
                    let parsed = parse_string(value).ok_or_else(|| {
                        format!(
                            "{}:{line_no}: package.name must be a quoted string",
                            file.display()
                        )
                    })?;
                    name = Some(parsed);
                }
            }
            s if is_dep_section(s) => {
                // "foo.workspace = true", "foo = "1.0"", "foo = { ... }": the
                // part before the first dot is the dependency name.
                let head = key.split('.').next().unwrap_or_default();
                let dep = dep_key(head, file, line_no)?;
                push_dep(&mut deps, s, dep);
            }
            _ => {}
        }
    }

    let name = name.ok_or_else(|| {
        format!(
            "{}: missing [package] name — manifest is not a valid member",
            file.display()
        )
    })?;
    Ok(Manifest { name, deps })
}

/// Parses a double-quoted TOML string value.
fn parse_string(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_owned())
}

/// Repository root: the parent of this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Member manifests under "crates/" and "apps/", in stable path order. The
/// root manifest and xtask itself are not member manifests.
pub fn workspace_manifests(root: &Path) -> Result<Vec<(PathBuf, Manifest)>, String> {
    let mut out = Vec::new();
    for group in ["crates", "apps"] {
        let dir = root.join(group);
        let entries = fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .map(|path| path.join("Cargo.toml"))
            .filter(|path| path.exists())
            .collect();
        paths.sort();
        for path in paths {
            let src =
                fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            let manifest = parse_manifest(&src, &path)?;
            out.push((path, manifest));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Manifest {
        parse_manifest(src, Path::new("Cargo.toml")).expect("parse must succeed")
    }

    #[test]
    fn parses_package_name_and_dependencies() {
        let manifest = parse(
            "[package]\n\
             name = \"reasonix-agent\"\n\
             version.workspace = true\n\
             \n\
             [dependencies]\n\
             reasonix-core.workspace = true\n\
             serde = \"1\"\n\
             \n\
             [dev-dependencies]\n\
             reasonix-testkit.workspace = true\n\
             \n\
             [target.'cfg(unix)'.dependencies]\n\
             libc = \"0.2\"\n",
        );
        assert_eq!(manifest.name, "reasonix-agent");
        assert_eq!(
            manifest.deps,
            vec![
                (
                    "dependencies".to_owned(),
                    vec![
                        "reasonix-core".to_owned(),
                        "serde".to_owned(),
                        "libc".to_owned(),
                    ],
                ),
                (
                    "dev-dependencies".to_owned(),
                    vec!["reasonix-testkit".to_owned()]
                ),
            ]
        );
    }

    #[test]
    fn parses_sub_table_dependency_headers() {
        let manifest = parse("[package]\nname = \"x\"\n\n[dependencies.foo]\nversion = \"1\"\n");
        assert_eq!(
            manifest.deps,
            vec![("dependencies".to_owned(), vec!["foo".to_owned()])]
        );
    }

    #[test]
    fn dotted_keys_yield_the_dependency_name() {
        let manifest =
            parse("[package]\nname = \"x\"\n[dependencies]\nreasonix-core.workspace = true\n");
        assert_eq!(manifest.deps[0].1, ["reasonix-core"]);
    }

    #[test]
    fn inline_tables_are_accepted() {
        let manifest =
            parse("[package]\nname = \"x\"\n[dependencies]\nfoo = { path = \"../foo\" }\n");
        assert_eq!(manifest.deps[0].1, ["foo"]);
    }

    #[test]
    fn multi_line_inline_tables_fail_closed() {
        let err = parse_manifest(
            "[package]\nname = \"x\"\n[dependencies]\nfoo = {\n  path = \"../foo\"\n}\n",
            Path::new("Cargo.toml"),
        )
        .expect_err("must fail");
        assert!(err.contains("multi-line"), "unexpected error: {err}");
    }

    #[test]
    fn missing_package_name_fails() {
        let err = parse_manifest("[dependencies]\nfoo = \"1\"\n", Path::new("Cargo.toml"))
            .expect_err("must fail");
        assert!(
            err.contains("missing [package] name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn duplicate_dependency_keys_deduplicate() {
        let manifest = parse(
            "[package]\nname = \"x\"\n[dependencies]\nfoo.workspace = true\nfoo = { path = \"../foo\" }\n",
        );
        assert_eq!(manifest.deps[0].1, ["foo"]);
    }

    #[test]
    fn invalid_keys_fail_closed() {
        let err = parse_manifest(
            "[package]\nname = \"x\"\n[dependencies]\nfoo bar = \"1\"\n",
            Path::new("Cargo.toml"),
        )
        .expect_err("must fail");
        assert!(
            err.contains("invalid dependency key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn array_of_tables_sections_are_ignored() {
        let manifest = parse(
            "[package]\nname = \"reasonix\"\n[[bin]]\nname = \"extra\"\npath = \"src/extra.rs\"\n",
        );
        assert_eq!(manifest.name, "reasonix");
        assert!(manifest.deps.is_empty());
    }

    #[test]
    fn non_dependency_sections_do_not_leak_keys() {
        let manifest = parse("[package]\nname = \"x\"\n[lints.rust]\nunsafe_code = \"forbid\"\n");
        assert!(manifest.deps.is_empty());
    }
}
