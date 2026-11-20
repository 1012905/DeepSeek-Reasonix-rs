// The window frame's scheme and ground. Windows draws a frameless window's
// border from the theme, and the ground is what shows wherever the page does
// not paint — so both follow the page, not the system, once the page says.
use crate::prefs::Prefs;
use tauri::window::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paint {
    pub mode: String,
    pub scheme: String,
}

/// The page's own `--page` ground for each scheme (tokens.css), as sRGB.
pub const GROUND_LIGHT: Color = Color(0xf4, 0xf1, 0xea, 0xff);
pub const GROUND_DARK: Color = Color(0x06, 0x0a, 0x10, 0xff);

/// What the page may tell the shell: a mode it knows, a scheme that is one of
/// two, and a fixed mode that agrees with its scheme.
pub fn validate(mode: &str, scheme: &str) -> Option<Paint> {
    if !matches!(mode, "auto" | "scheduled" | "light" | "dark")
        || !matches!(scheme, "light" | "dark")
    {
        return None;
    }
    if matches!(mode, "light" | "dark") && mode != scheme {
        return None;
    }
    Some(Paint {
        mode: mode.into(),
        scheme: scheme.into(),
    })
}

fn fixed(value: Option<&str>) -> Source {
    match value {
        Some("dark") => Source::Dark,
        Some("light") => Source::Light,
        _ => Source::System,
    }
}

pub fn source(prefs: &Prefs, painted: Option<&Paint>) -> Source {
    let mode = painted
        .map(|p| p.mode.as_str())
        .or(prefs.get("rx-theme").map(String::as_str));
    if mode == Some("scheduled") {
        let scheme = painted
            .map(|p| p.scheme.as_str())
            .or(prefs.get("rx-theme-resolved").map(String::as_str));
        return fixed(scheme);
    }
    fixed(mode)
}

pub fn ground(source: Source, system_dark: bool) -> Color {
    let dark = match source {
        Source::System => system_dark,
        Source::Dark => true,
        Source::Light => false,
    };
    if dark { GROUND_DARK } else { GROUND_LIGHT }
}

pub fn theme(source: Source) -> Option<tauri::Theme> {
    match source {
        Source::System => None,
        Source::Light => Some(tauri::Theme::Light),
        Source::Dark => Some(tauri::Theme::Dark),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(pairs: &[(&str, &str)]) -> Prefs {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn validate_refuses_a_fixed_mode_that_disagrees() {
        assert!(validate("dark", "dark").is_some());
        assert!(validate("auto", "light").is_some());
        assert!(validate("scheduled", "dark").is_some());
        assert!(validate("dark", "light").is_none());
        assert!(validate("neon", "dark").is_none());
        assert!(validate("auto", "grey").is_none());
    }

    #[test]
    fn source_prefers_what_the_page_painted() {
        let saved = prefs(&[("rx-theme", "light")]);
        assert_eq!(source(&saved, None), Source::Light);
        let painted = Paint {
            mode: "dark".into(),
            scheme: "dark".into(),
        };
        assert_eq!(source(&saved, Some(&painted)), Source::Dark);
        assert_eq!(source(&Prefs::new(), None), Source::System);
    }

    #[test]
    fn a_schedule_takes_its_resolved_scheme() {
        let saved = prefs(&[("rx-theme", "scheduled"), ("rx-theme-resolved", "dark")]);
        assert_eq!(source(&saved, None), Source::Dark);
        let unresolved = prefs(&[("rx-theme", "scheduled")]);
        assert_eq!(source(&unresolved, None), Source::System);
    }

    #[test]
    fn ground_follows_the_system_only_when_the_page_has_not_said() {
        assert_eq!(ground(Source::System, true), GROUND_DARK);
        assert_eq!(ground(Source::System, false), GROUND_LIGHT);
        assert_eq!(ground(Source::Light, true), GROUND_LIGHT);
    }
}
