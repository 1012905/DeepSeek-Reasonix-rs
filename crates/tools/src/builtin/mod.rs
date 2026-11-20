//! The read-only builtin tool slice (wave 19 plan, slice 2): read_file,
//! glob, ls, grep, and the read-side half of the workspace confinement
//! machinery (confine.go). Parity targets live in each module's docs; the
//! write-side machines the confine constructors also bind are anchored in
//! [`confine::anchors`], not forged.

#![forbid(unsafe_code)]

pub mod anchors;
pub mod confine;
pub mod gitignore;
pub mod glob;
pub mod globengine;
pub mod grep;
pub mod listdir;
pub mod network;
pub mod pathgo;
pub mod readfile;
pub mod secretpaths;
pub mod walk;

pub use anchors::{CODE_WRITE_OUTSIDE_SCOPE, ConfineError, confine_write_side_resolved};
pub use confine::{
    CODE_READ_OUTSIDE_SCOPE, confine, confine_opened, confine_read, confine_resolved,
    confine_scope, read_outside_scope, real_path, real_roots, within, within_fold,
};
pub use glob::{
    GLOB_DEFAULT_TIMEOUT, GLOB_MAX_RESULTS, GLOB_MAX_TIMEOUT, GlobTool, glob_base, glob_walk_root,
};
pub use grep::{
    GREP_DEFAULT_TIMEOUT, GREP_MAX_MATCHES, GREP_MAX_TIMEOUT, GrepScope, GrepTool, confine_search,
    format_grep, grep_glob_matches, resolve_search, timeout as tool_timeout,
};
pub use listdir::ListDirTool;
pub use network::{
    CODE_NETWORK_PATH_OUTSIDE_SCOPE, ResolvedPath, is_network_path, refuse_network_path,
};
pub use readfile::{READ_FILE_DEFAULT_LIMIT, ReadFileTool};
pub use secretpaths::{protect_sensitive_files, set_protect_sensitive_files};

use std::sync::Arc;

use crate::abi::Tool;

/// confine.go `ConfineReaders` for the readers this slice migrated: the
/// read/list/search built-ins bound to forbidRoots — directories the agent
/// may not read or list. Empty forbidRoots yields unconfined readers. The
/// fourth Go member, code_index, is not part of this slice and lands with
/// its own ledger row.
pub fn confine_readers(forbid_roots: &[String]) -> Vec<Arc<dyn Tool>> {
    let rs = real_roots(forbid_roots);
    vec![
        Arc::new(ReadFileTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
        Arc::new(ListDirTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
        Arc::new(GlobTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
    ]
}

/// The unconfined instances a composition root overrides, mirroring the Go
/// init-time registrations for the readers of this slice.
pub fn unconfined_readers() -> Vec<Arc<dyn Tool>> {
    confine_readers(&[])
}
