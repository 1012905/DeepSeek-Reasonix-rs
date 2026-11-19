//! Top-level `language` key and its normalization matrix.

/// Normalizes the ui/model language setting to `auto`|`zh`|`en`
/// (config.go NormalizeLanguage). Unrecognized tags fall back to auto.
pub fn normalize_language(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" | "detect" | "default" => "auto",
        "zh" | "cn" | "chinese" | "中文" => "zh",
        "en" | "english" => "en",
        _ => "auto",
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_language;

    #[test]
    fn empty_and_auto_aliases_resolve_to_auto() {
        // config.md §3: ""|auto|detect|default → auto.
        for raw in ["", "auto", "detect", "default", "  AUTO  "] {
            assert_eq!(normalize_language(raw), "auto", "raw = {raw:?}");
        }
    }

    #[test]
    fn zh_family_resolves_to_zh() {
        for raw in ["zh", "CN", "chinese", "中文"] {
            assert_eq!(normalize_language(raw), "zh", "raw = {raw:?}");
        }
    }

    #[test]
    fn en_family_resolves_to_en() {
        for raw in ["en", "ENGLISH"] {
            assert_eq!(normalize_language(raw), "en", "raw = {raw:?}");
        }
    }

    #[test]
    fn unrecognized_falls_back_to_auto() {
        // config.md §3: unknown → auto; the raw value is never preserved.
        // "zh-tw" is a locale, not one of the four zh spellings.
        assert_eq!(normalize_language("fr"), "auto");
        assert_eq!(normalize_language("zh-tw"), "auto");
    }
}
