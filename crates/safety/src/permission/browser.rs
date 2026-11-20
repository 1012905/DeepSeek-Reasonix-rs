//! Typing a secret into a site, and the subjects a browser call answers to.

/// Marks a browser subject that enters a secret — a password, a one-time
/// code, card details — on the site after it. The host writes it; only a rule
/// naming that exact subject answers it, never a site grant or a glob, and
/// auto does not.
pub const BROWSER_CREDENTIAL_PREFIX: &str = "credential:";

/// Marks arbitrary page JavaScript that only an exact grant answers.
pub const BROWSER_SCRIPT_PREFIX: &str = "script:";

/// Reports a browser subject a person has to answer unless YOLO or an exact
/// grant already did.
pub fn browser_subject_requires_explicit_approval(subject: &str) -> bool {
    subject.starts_with(BROWSER_CREDENTIAL_PREFIX) || subject.starts_with(BROWSER_SCRIPT_PREFIX)
}

/// Every subject a browser call answers to: a credential entry is also a
/// visit to its site, so a rule refusing the site refuses it.
pub(crate) fn browser_subjects(origin: &str) -> Vec<String> {
    for prefix in [BROWSER_CREDENTIAL_PREFIX, BROWSER_SCRIPT_PREFIX] {
        if let Some(site) = origin.strip_prefix(prefix)
            && !site.is_empty()
        {
            return vec![origin.to_string(), site.to_string()];
        }
    }
    vec![origin.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_subject_expands_to_site() {
        let got = browser_subjects("credential:https://bank.example");
        assert_eq!(
            got,
            vec!["credential:https://bank.example", "https://bank.example"]
        );
        assert_eq!(
            browser_subjects("https://example.com"),
            vec!["https://example.com"]
        );
        assert!(browser_subject_requires_explicit_approval("credential:x"));
        assert!(browser_subject_requires_explicit_approval("script:x"));
        assert!(!browser_subject_requires_explicit_approval("https://x"));
    }
}
