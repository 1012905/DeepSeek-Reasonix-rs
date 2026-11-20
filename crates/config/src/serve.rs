//! The [serve] section: HTTP frontend security settings, their auth-mode
//! normalization, and the host's credential-free view (HOST-018).

use serde::{Deserialize, Deserializer};

use crate::model::Config;

/// Auth mode for the serve frontend, normalized from the raw `auth_mode`
/// string at load: trimmed and lowercased, empty = none, anything outside
/// none|token|password is invalid (serve/auth.go authInvalid denies every
/// request rather than guessing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// No authentication; mutations still need the launch token.
    #[default]
    None,
    /// Pre-shared token in the URL query string or cookie.
    Token,
    /// Login page with bcrypt password verification.
    Password,
    /// Unrecognized mode: the gate refuses everything (auth.go authInvalid).
    Invalid,
}

impl AuthMode {
    /// Normalizes one raw `auth_mode` value (serve/auth.go NormalizeAuthMode).
    pub fn normalize(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "" | "none" => AuthMode::None,
            "token" => AuthMode::Token,
            "password" => AuthMode::Password,
            _ => AuthMode::Invalid,
        }
    }

    /// The mode name the gate reports: none|token|password|invalid.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::None => "none",
            AuthMode::Token => "token",
            AuthMode::Password => "password",
            AuthMode::Invalid => "invalid",
        }
    }
}

// A non-string auth_mode must fail the whole layer (BurntSushi surfaces a
// type error and the reference drops that file), so this is not a visitor
// that maps garbage to Invalid.
impl<'de> Deserialize<'de> for AuthMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(AuthMode::normalize(&raw))
    }
}

/// [serve]: HTTP frontend security. User-global — a workspace file never gets
/// to set it (load_project.go userGlobals restores the user's values).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct ServeSection {
    pub auth_mode: AuthMode,
    /// Pre-shared token for token mode; empty = generated at startup.
    pub token: String,
    /// bcrypt hash of the password for password mode.
    pub password_hash: String,
    /// Trust X-Forwarded-For/Proto from a reverse proxy the user runs.
    pub behind_proxy: bool,
    /// Fixed phone-access LAN port; 0 picks a free one each time.
    pub share_port: i64,
    /// Keep paired phones across restarts; off by default.
    pub remember_paired_devices: bool,
}

impl ServeSection {
    pub const MIN_SHARE_PORT: i64 = 1024;
    pub const MAX_SHARE_PORT: i64 = 65535;

    /// The configured share port, or 0 when unset or hand-written outside the
    /// unprivileged range (serve_share_port.go SharePort).
    pub fn share_port(&self) -> i64 {
        if self.share_port == 0
            || (Self::MIN_SHARE_PORT..=Self::MAX_SHARE_PORT).contains(&self.share_port)
        {
            self.share_port
        } else {
            0
        }
    }
}

/// The user's serve settings with their authentication taken out. Studio's
/// boundary is the loopback gate, and both gates read one cookie: a configured
/// token left in place would have the hub refuse the credential this launch
/// minted, on every request (hostServeConfig, HOST-018).
pub fn host_serve_section(config: &Config) -> ServeSection {
    let mut serve = config.serve.clone();
    serve.auth_mode = AuthMode::None;
    serve.token = String::new();
    serve.password_hash = String::new();
    serve.behind_proxy = false;
    serve
}

#[cfg(test)]
mod tests {
    use super::{AuthMode, ServeSection, host_serve_section};
    use crate::model::Config;

    #[test]
    fn auth_mode_normalization_matrix() {
        // serve/auth.go NormalizeAuthMode: lower/trim, "" = none.
        assert_eq!(AuthMode::normalize(""), AuthMode::None);
        assert_eq!(AuthMode::normalize("   "), AuthMode::None);
        assert_eq!(AuthMode::normalize("none"), AuthMode::None);
        assert_eq!(AuthMode::normalize("  TOKEN  "), AuthMode::Token);
        assert_eq!(AuthMode::normalize("Password"), AuthMode::Password);
        assert_eq!(AuthMode::normalize("bogus"), AuthMode::Invalid);
    }

    #[test]
    fn auth_mode_strings_match_the_gate_report() {
        // auth.go Mode(): the four names the hub can answer with.
        assert_eq!(AuthMode::None.as_str(), "none");
        assert_eq!(AuthMode::Token.as_str(), "token");
        assert_eq!(AuthMode::Password.as_str(), "password");
        assert_eq!(AuthMode::Invalid.as_str(), "invalid");
    }

    #[test]
    fn serve_defaults_are_zero() {
        // config.md §1/§4: zero ServeConfig; auth_mode "" reads as none.
        let serve = ServeSection::default();
        assert_eq!(serve.auth_mode, AuthMode::None);
        assert_eq!(serve.token, "");
        assert_eq!(serve.password_hash, "");
        assert!(!serve.behind_proxy);
        assert_eq!(serve.share_port(), 0);
        assert!(!serve.remember_paired_devices);
    }

    #[test]
    fn share_port_outside_the_unprivileged_range_reads_zero() {
        // serve_share_port.go: only 0 or 1024..=65535 survive the accessor.
        let serve = ServeSection {
            share_port: 8090,
            ..ServeSection::default()
        };
        assert_eq!(serve.share_port(), 8090);
        let serve = ServeSection {
            share_port: 80,
            ..ServeSection::default()
        };
        assert_eq!(serve.share_port(), 0);
        let serve = ServeSection {
            share_port: 70000,
            ..ServeSection::default()
        };
        assert_eq!(serve.share_port(), 0);
    }

    #[test]
    fn host_serve_section_zeroes_the_four_auth_fields_only() {
        // HOST-018: nothing in the user's configuration can open, weaken, or
        // predict the Studio boundary; unrelated keys must survive.
        let mut config = Config::default();
        config.serve.auth_mode = AuthMode::normalize("token");
        config.serve.token = "secret".to_owned();
        config.serve.password_hash = "bcrypt".to_owned();
        config.serve.behind_proxy = true;
        config.serve.share_port = 8090;
        config.serve.remember_paired_devices = true;

        let serve = host_serve_section(&config);
        assert_eq!(serve.auth_mode, AuthMode::None);
        assert_eq!(serve.token, "");
        assert_eq!(serve.password_hash, "");
        assert!(!serve.behind_proxy);
        assert_eq!(serve.share_port, 8090);
        assert!(serve.remember_paired_devices);
    }
}
