//! The `[secrets]` section (Go `SecretsConfig`): the denylist toggles the
//! read tools answer to. Keys this slice does not carry yet decode through
//! untouched, like every other table.

use serde::Deserialize;

/// Go `SecretsConfig`, the carried key. Default off: hiding the files breaks
/// legitimate "edit my .env" workflows; a user who opts in gets the
/// denylists armed at boot (Go build.go arms the package globals before any
/// subprocess can spawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct SecretsSection {
    /// Makes read/list/search tools treat credential paths (.env,
    /// .git-credentials, .netrc, *.pem/*.key/*.p12/*.pfx, ~/.ssh) as
    /// invisible.
    pub protect_sensitive_files: bool,
}
