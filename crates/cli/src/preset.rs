//! Runtime profile (`--preset` / deprecated `--profile`) parsing.
//!
//! Parity target: `internal/frontend/cli/cli.go:260-273` (`parseRuntimeProfile`).
//! The accepted spellings map onto the three dual-write token modes; unknown
//! values fail with the reference error text quoting the raw input.

use crate::text::PRESET_MENU_OR;

/// Dual-write token mode a runtime profile selects (`boot.TokenMode*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenMode {
    Full,
    Economy,
    Delivery,
}

impl TokenMode {
    /// Canonical persisted spelling (`full`, `economy`, `delivery`).
    pub fn as_str(self) -> &'static str {
        match self {
            TokenMode::Full => "full",
            TokenMode::Economy => "economy",
            TokenMode::Delivery => "delivery",
        }
    }
}

/// Parses a raw `--preset`/`--profile` value. Matching trims space and folds
/// case for the lookup; the error quotes the original, un-normalized value.
pub fn parse_runtime_profile(value: &str) -> Result<TokenMode, String> {
    let normalized = value.trim().to_lowercase();
    let mode = match normalized.as_str() {
        "" | "balanced" | "full" => TokenMode::Full,
        "economy" | "light" | "lite" | "eco" => TokenMode::Economy,
        "delivery" | "deliver" | "quality" => TokenMode::Delivery,
        _ => {
            return Err(format!(
                "unknown execution setting {:?} (want {PRESET_MENU_OR}; legacy: economy, full)",
                value
            ));
        }
    };
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_matrix_maps_accepted_spellings() {
        let cases: [(&str, TokenMode); 13] = [
            ("", TokenMode::Full),
            ("balanced", TokenMode::Full),
            ("full", TokenMode::Full),
            ("economy", TokenMode::Economy),
            ("light", TokenMode::Economy),
            ("lite", TokenMode::Economy),
            ("eco", TokenMode::Economy),
            ("delivery", TokenMode::Delivery),
            ("deliver", TokenMode::Delivery),
            ("quality", TokenMode::Delivery),
            ("  Balanced ", TokenMode::Full),
            ("ECO", TokenMode::Economy),
            ("DELIVERY", TokenMode::Delivery),
        ];
        for (raw, want) in cases {
            assert_eq!(parse_runtime_profile(raw).unwrap(), want, "{raw}");
        }
    }

    #[test]
    fn unknown_profile_quotes_raw_text() {
        let err = parse_runtime_profile("FAST").unwrap_err();
        assert_eq!(
            err,
            "unknown execution setting \"FAST\" (want balanced or delivery; legacy: economy, full)"
        );
    }
}
