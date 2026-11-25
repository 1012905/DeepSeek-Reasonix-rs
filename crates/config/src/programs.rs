//! Project-program approval gates and content digests (docs/contracts/config.md
//! §10.17; project_programs.go, project_program_verify.go).
//!
//! A project checkout can name host-run programs (hooks, language servers,
//! ripgrep, the shell, the browser, the Laya Python runtime). Each runs only
//! once the user approved that exact declaration: approvals live under the
//! user's home, never in the checkout, keyed by workspace fingerprint, kind,
//! name and content digest.
//!
//! This slice ports the digest, the approval-match decision, the in-memory
//! approval record and the word extraction. Fingerprinting, the JSON store,
//! the cross-process lock and the boot call site are wiring left to the
//! integration layer (CONFIG-009 wip).

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Hooks declared by the checkout.
pub const KIND_HOOKS: &str = "hooks";
/// A language server entry.
pub const KIND_LSP: &str = "lsp";
/// The ripgrep executable path.
pub const KIND_RG_PATH: &str = "rg_path";
/// The shell executable path.
pub const KIND_SHELL_PATH: &str = "shell_path";
/// The browser executable.
pub const KIND_BROWSER: &str = "browser";
/// The Laya Python runtime.
pub const KIND_LAYA_PYTHON: &str = "laya_python";

/// Why an approval decision or digest verification could not be completed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProgramError {
    /// The approval record could not be read or written (store IO = wiring).
    #[error("project program approvals unavailable")]
    Unavailable,
    /// An approved program's files changed after the approval was checked.
    #[error("workspace program changed since it was approved: {kind} {name:?}")]
    Changed { kind: String, name: String },
    /// The declaration body could not be serialized.
    #[error("marshal program declaration: {message}")]
    Declaration { message: String },
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Lowercase hex of a SHA-256 digest. The table is the only hand-written part;
/// the hash itself comes from the `sha2` crate.
fn hex_lower(sum: &[u8]) -> String {
    let mut out = String::with_capacity(sum.len() * 2);
    for byte in sum {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// One file a program's declaration names, with its in-memory content.
/// `None` models an unreadable file: it hashes differently from its approved
/// self rather than being treated as empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramFile {
    pub path: String,
    pub content: Option<Vec<u8>>,
}

/// The SHA-256 hex of a declaration together with every named file's content
/// (project_programs.go currentDigest). The stream is the declaration bytes,
/// then for each file: NUL + path + NUL + content; an unreadable file appends
/// NUL + "unreadable" instead of content.
pub fn current_digest(declaration: &[u8], files: &[ProgramFile]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(declaration);
    for file in files {
        hasher.update([0u8]);
        hasher.update(file.path.as_bytes());
        hasher.update([0u8]);
        match &file.content {
            Some(content) => hasher.update(content),
            None => hasher.update(b"\x00unreadable"),
        }
    }
    hex_lower(&hasher.finalize())
}

/// Builds the declaration body a digest covers: `{"Kind":…,"Name":…,"Decl":…}`
/// (project_programs.go NewProjectProgram). Struct field order matches Go's
/// struct marshal so the byte stream matches byte-for-byte.
pub fn declaration_body(kind: &str, name: &str, decl: &Value) -> Result<String, ProgramError> {
    #[derive(serde::Serialize)]
    struct Body<'a> {
        #[serde(rename = "Kind")]
        kind: &'a str,
        #[serde(rename = "Name")]
        name: &'a str,
        #[serde(rename = "Decl")]
        decl: &'a Value,
    }
    serde_json::to_string(&Body { kind, name, decl }).map_err(|error| ProgramError::Declaration {
        message: error.to_string(),
    })
}

/// One project-declared program as the user would approve it. `Detail` is the
/// one-line summary shown for approval; `Declaration` and `Files` are
/// everything `Digest` covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProgram {
    pub kind: String,
    pub name: String,
    pub detail: String,
    pub declaration: String,
    pub files: Vec<String>,
    pub digest: String,
}

impl ProjectProgram {
    /// Verifies the files still hold the approved content
    /// (project_program_verify.go Verify). A program with no named files has
    /// nothing to drift; a digest mismatch reports the changed error.
    pub fn verify(&self, files: &[ProgramFile]) -> Result<(), ProgramError> {
        if files.is_empty() || current_digest(self.declaration.as_bytes(), files) == self.digest {
            Ok(())
        } else {
            Err(ProgramError::Changed {
                kind: self.kind.clone(),
                name: self.name.clone(),
            })
        }
    }
}

/// An approval recorded for one workspace folder (project_programs.go
/// programApproval).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramApproval {
    pub workspace: String,
    pub kind: String,
    pub name: String,
    pub digest: String,
}

/// In-memory approval record. The JSON store, lock and home-path resolution
/// live in the wiring layer; this is the pure state the store mutates under
/// lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramStore {
    approvals: Vec<ProgramApproval>,
}

impl ProgramStore {
    /// Creates an empty record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports whether `program`, exactly as digested, was approved for `ws`
    /// (project_programs.go Approved). An empty workspace never matches.
    pub fn approved(&self, ws: &str, program: &ProjectProgram) -> bool {
        if ws.is_empty() {
            return false;
        }
        self.approvals.iter().any(|approval| {
            approval.workspace == ws
                && approval.kind == program.kind
                && approval.name == program.name
                && approval.digest == program.digest
        })
    }

    /// Records programs for `ws`, replacing the earlier digest of the same
    /// program (project_programs.go Approve).
    pub fn approve(&mut self, ws: &str, programs: &[ProjectProgram]) {
        if ws.is_empty() {
            return;
        }
        self.approvals.retain(|approval| {
            !programs.iter().any(|program| {
                approval.workspace == ws
                    && approval.kind == program.kind
                    && approval.name == program.name
            })
        });
        for program in programs {
            self.approvals.push(ProgramApproval {
                workspace: ws.to_owned(),
                kind: program.kind.clone(),
                name: program.name.clone(),
                digest: program.digest.clone(),
            });
        }
    }

    /// Drops every approval recorded for `ws` (project_programs.go Revoke).
    pub fn revoke(&mut self, ws: &str) {
        self.approvals.retain(|approval| approval.workspace != ws);
    }
}

/// Yields each flag word and, for a `--flag=./x` style word, its value
/// (project_programs.go namedWords).
pub fn named_words(words: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for word in words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        out.push(word.to_owned());
        if let Some((_, value)) = word.split_once('=')
            && !value.trim().is_empty()
        {
            out.push(value.trim().to_owned());
        }
    }
    out
}

/// Renders env for a person approving it, in a stable key order
/// (project_programs.go EnvSummary).
pub fn env_summary(env: &BTreeMap<String, String>) -> String {
    let mut summary = String::new();
    for (key, value) in env {
        summary.push_str(&format!(" {key}={value}"));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(kind: &str, name: &str, declaration: &str, digest: &str) -> ProjectProgram {
        ProjectProgram {
            kind: kind.to_owned(),
            name: name.to_owned(),
            detail: name.to_owned(),
            declaration: declaration.to_owned(),
            files: Vec::new(),
            digest: digest.to_owned(),
        }
    }

    #[test]
    fn digest_matches_the_sha256_official_vectors() {
        // sha2 crate output against the published NIST vectors; the hex table
        // must render lowercase two-digit bytes.
        assert_eq!(
            current_digest(b"", &[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            current_digest(b"abc", &[]),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn digest_covers_declaration_and_each_file_with_nul_separators() {
        let files = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: Some(b"print(1)".to_vec()),
        }];
        let with_file = current_digest(b"decl", &files);
        let no_file = current_digest(b"decl", &[]);
        assert_ne!(with_file, no_file);

        // The file path and content both enter the stream.
        let other_content = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: Some(b"print(2)".to_vec()),
        }];
        assert_ne!(with_file, current_digest(b"decl", &other_content));

        // An unreadable file hashes as NUL+unreadable, not as empty content.
        let unreadable = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: None,
        }];
        let empty_content = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: Some(Vec::new()),
        }];
        assert_ne!(
            current_digest(b"decl", &unreadable),
            current_digest(b"decl", &empty_content)
        );
    }

    #[test]
    fn digest_follows_file_order_and_declaration_bytes() {
        let files = vec![
            ProgramFile {
                path: "a".to_owned(),
                content: Some(b"1".to_vec()),
            },
            ProgramFile {
                path: "b".to_owned(),
                content: Some(b"2".to_vec()),
            },
        ];
        let reversed = vec![
            ProgramFile {
                path: "b".to_owned(),
                content: Some(b"2".to_vec()),
            },
            ProgramFile {
                path: "a".to_owned(),
                content: Some(b"1".to_vec()),
            },
        ];
        // Same set, different order: the stream differs, so the digest differs.
        assert_ne!(
            current_digest(b"decl", &files),
            current_digest(b"decl", &reversed)
        );

        // Whitespace and casing in the declaration change the covered bytes.
        assert_ne!(
            current_digest(b"decl", &files),
            current_digest(b" decl", &files)
        );
        assert_ne!(
            current_digest(b"decl", &files),
            current_digest(b"DECL", &files)
        );
    }

    #[test]
    fn declaration_body_keeps_go_field_order_and_inlines_the_decl() {
        let body = declaration_body(
            KIND_LSP,
            "rust",
            &serde_json::json!({"command": "rust-analyzer"}),
        )
        .expect("body");
        assert_eq!(
            body,
            r#"{"Kind":"lsp","Name":"rust","Decl":{"command":"rust-analyzer"}}"#
        );
    }

    #[test]
    fn approval_matches_the_four_tuple_and_replaces_by_program_identity() {
        let mut store = ProgramStore::new();
        let first = program(KIND_SHELL_PATH, "shell", "decl-1", "digest-1");
        store.approve("ws", std::slice::from_ref(&first));
        assert!(store.approved("ws", &first));

        // A different workspace, kind, name or digest does not match.
        assert!(!store.approved("other-ws", &first));
        let other_kind = program(KIND_RG_PATH, "shell", "decl-1", "digest-1");
        assert!(!store.approved("ws", &other_kind));
        let other_name = program(KIND_SHELL_PATH, "rg", "decl-1", "digest-1");
        assert!(!store.approved("ws", &other_name));
        let other_digest = program(KIND_SHELL_PATH, "shell", "decl-2", "digest-2");
        assert!(!store.approved("ws", &other_digest));

        // Re-approving the same program replaces its earlier digest only.
        store.approve("ws", std::slice::from_ref(&other_digest));
        assert!(!store.approved("ws", &first));
        assert!(store.approved("ws", &other_digest));

        // Revoke drops every approval for the workspace only.
        store.approve("ws-2", std::slice::from_ref(&first));
        store.revoke("ws");
        assert!(!store.approved("ws", &other_digest));
        assert!(store.approved("ws-2", &first));
    }

    #[test]
    fn empty_workspace_never_matches_an_approval() {
        let mut store = ProgramStore::new();
        store.approve("", &[program(KIND_HOOKS, "hook", "d", "dig")]);
        assert!(!store.approved("", &program(KIND_HOOKS, "hook", "d", "dig")));
    }

    #[test]
    fn verify_passes_when_files_match_and_flags_drift() {
        let files = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: Some(b"print(1)".to_vec()),
        }];
        let digest = current_digest(b"decl", &files);
        let approved = ProjectProgram {
            kind: KIND_LAYA_PYTHON.to_owned(),
            name: "laya".to_owned(),
            detail: "laya".to_owned(),
            declaration: "decl".to_owned(),
            files: vec!["a.py".to_owned()],
            digest,
        };
        assert!(approved.verify(&files).is_ok());

        let changed = vec![ProgramFile {
            path: "a.py".to_owned(),
            content: Some(b"print(2)".to_vec()),
        }];
        let err = approved.verify(&changed).expect_err("drift refused");
        assert!(matches!(err, ProgramError::Changed { .. }));

        // No named files: nothing can drift.
        let fileless = program(KIND_HOOKS, "hook", "decl", "dig");
        assert!(fileless.verify(&[]).is_ok());
    }

    #[test]
    fn named_words_also_yield_flag_values() {
        let words = vec![
            "npx".to_owned(),
            "--require=./a.js".to_owned(),
            "  ".to_owned(),
            "--flag=  ".to_owned(),
        ];
        assert_eq!(
            named_words(&words),
            vec!["npx", "--require=./a.js", "./a.js", "--flag="]
        );
    }

    #[test]
    fn env_summary_renders_sorted_pairs() {
        let mut env = BTreeMap::new();
        env.insert("b".to_owned(), "2".to_owned());
        env.insert("a".to_owned(), "1".to_owned());
        assert_eq!(env_summary(&env), " a=1 b=2");
        assert_eq!(env_summary(&BTreeMap::new()), "");
    }
}
