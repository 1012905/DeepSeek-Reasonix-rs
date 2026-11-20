//! Reading and operating another application on this machine.

/// Marks a subject whose steps move the person's own pointer rather than
/// asking an element to act. The host writes it; a grant for the application
/// does not answer it, because the two are different things to agree to.
pub const COMPUTER_POINTER_PREFIX: &str = "pointer:";

/// Reports a subject a person answers for even when the application it names
/// is already granted.
pub fn computer_subject_takes_pointer(subject: &str) -> bool {
    subject.starts_with(COMPUTER_POINTER_PREFIX)
}

/// Every subject a computer call answers to: taking the pointer over an
/// application is also operating that application, so a rule refusing the
/// application refuses it.
pub(crate) fn computer_subjects(subject: &str) -> Vec<String> {
    if let Some(app) = subject.strip_prefix(COMPUTER_POINTER_PREFIX)
        && !app.is_empty()
    {
        return vec![subject.to_string(), app.to_string()];
    }
    vec![subject.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_subject_expands_to_application() {
        let got = computer_subjects("pointer:com.apple.Notes");
        assert_eq!(got, vec!["pointer:com.apple.Notes", "com.apple.Notes"]);
        assert_eq!(
            computer_subjects("com.apple.Notes"),
            vec!["com.apple.Notes"]
        );
        assert!(computer_subject_takes_pointer("pointer:x"));
        assert!(!computer_subject_takes_pointer("com.apple.Notes"));
    }
}
