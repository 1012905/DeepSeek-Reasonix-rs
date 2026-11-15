// Where a link may go. Links come out of model output, so the platform opener
// is never handed a scheme of the link's choosing — file: and whatever handlers
// a machine has registered are reached through exactly this.
pub fn external_target(raw: &str) -> Option<String> {
    let url: tauri::Url = raw.parse().ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_leave() {
        assert_eq!(
            external_target("https://reasonix.io/a?b=1").as_deref(),
            Some("https://reasonix.io/a?b=1")
        );
        assert_eq!(
            external_target("http://example.com").as_deref(),
            Some("http://example.com/")
        );
        assert_eq!(external_target("file:///etc/passwd"), None);
        assert_eq!(external_target("ms-settings:privacy"), None);
        assert_eq!(external_target("javascript:alert(1)"), None);
        assert_eq!(external_target("not a url"), None);
    }
}
