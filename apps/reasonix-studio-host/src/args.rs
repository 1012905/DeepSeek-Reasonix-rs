//! Host flags with Go `flag`-package semantics for the frozen flag set
//! (docs/contracts/host.md §1.2): `-name value`, `-name=value`, bool flags
//! that never consume the next token, and exit 2 on an undefined flag.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub page: PathBuf,
    pub instance_id: bool,
    pub studio_version: String,
    pub studio_app: PathBuf,
    pub studio_app_pid: i64,
    pub cua_driver: PathBuf,
    pub computer_abort_socket: String,
    pub computer_socket: String,
    pub crash_dir: PathBuf,
    pub strip_package_grants: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("flag provided but not defined: -{0}")]
    Undefined(String),
    #[error("flag needs an argument: -{0}")]
    MissingValue(String),
    #[error("invalid value \"{1}\" for -{0}")]
    BadValue(String, String),
}

pub const USAGE: &str = "usage: reasonix-studio-host [-page dir] [-instance-id] \
[-studio-version v] [-studio-app exe] [-studio-app-pid pid] [-cua-driver exe] \
[-computer-abort-socket path] [-computer-socket path] [-crash-dir dir] [-strip-package-grants]";

/// Parses argv the way Go's flag package does for this set: one or two leading
/// dashes, parsing stops at the first non-flag argument, `--` ends the flags.
pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, ParseError> {
    let mut args = Args::default();
    let mut it = argv.into_iter().peekable();
    while let Some(raw) = it.next() {
        if raw == "--" {
            break;
        }
        // Go's flag package: one or two dashes introduce a flag; a bare "-" or
        // anything with a third dash is a positional argument that stops parsing.
        let stripped = raw.strip_prefix("--").or_else(|| raw.strip_prefix('-'));
        let body = match stripped {
            Some(b) if !b.is_empty() && !b.starts_with('-') => b,
            _ => break,
        };
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (body, None),
        };
        match name {
            "instance-id" => args.instance_id = bool_value(name, inline, &mut it)?,
            "strip-package-grants" => {
                args.strip_package_grants = bool_value(name, inline, &mut it)?
            }
            "page" => args.page = PathBuf::from(text_value(name, inline, &mut it)?),
            "studio-version" => args.studio_version = text_value(name, inline, &mut it)?,
            "studio-app" => args.studio_app = PathBuf::from(text_value(name, inline, &mut it)?),
            "studio-app-pid" => {
                args.studio_app_pid = text_value(name, inline, &mut it)?
                    .parse::<i64>()
                    .map_err(|_| ParseError::BadValue(name.to_string(), "pid".to_string()))?
            }
            "cua-driver" => args.cua_driver = PathBuf::from(text_value(name, inline, &mut it)?),
            "computer-abort-socket" => {
                args.computer_abort_socket = text_value(name, inline, &mut it)?
            }
            "computer-socket" => args.computer_socket = text_value(name, inline, &mut it)?,
            "crash-dir" => args.crash_dir = PathBuf::from(text_value(name, inline, &mut it)?),
            _ => return Err(ParseError::Undefined(name.to_string())),
        }
    }
    Ok(args)
}

fn bool_value<I: Iterator<Item = String>>(
    name: &str,
    inline: Option<String>,
    _it: &mut I,
) -> Result<bool, ParseError> {
    match inline {
        Some(v) => match v.as_str() {
            "true" | "1" | "t" | "T" | "TRUE" | "True" => Ok(true),
            "false" | "0" | "f" | "F" | "FALSE" | "False" => Ok(false),
            _ => Err(ParseError::BadValue(name.to_string(), v)),
        },
        None => Ok(true),
    }
}

fn text_value<I: Iterator<Item = String>>(
    name: &str,
    inline: Option<String>,
    it: &mut I,
) -> Result<String, ParseError> {
    match inline {
        Some(v) => Ok(v),
        None => it
            .next()
            .ok_or_else(|| ParseError::MissingValue(name.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_dash_dash_and_space_forms() {
        let a = parse(argv(&["-page", "dist", "--studio-version", "2.10.0"])).unwrap();
        assert_eq!(a.page, PathBuf::from("dist"));
        assert_eq!(a.studio_version, "2.10.0");
    }

    #[test]
    fn parses_inline_values() {
        let a = parse(argv(&["-page=dist", "-studio-app-pid=42"])).unwrap();
        assert_eq!(a.page, PathBuf::from("dist"));
        assert_eq!(a.studio_app_pid, 42);
    }

    #[test]
    fn bool_flags_never_consume_the_next_token() {
        let a = parse(argv(&["-instance-id", "-page", "dist"])).unwrap();
        assert!(a.instance_id);
        assert_eq!(a.page, PathBuf::from("dist"));
        let b = parse(argv(&["-strip-package-grants=false"])).unwrap();
        assert!(!b.strip_package_grants);
    }

    #[test]
    fn undefined_flag_is_the_exit_two_shape() {
        assert_eq!(
            parse(argv(&["-bogus"])),
            Err(ParseError::Undefined("bogus".to_string()))
        );
    }

    #[test]
    fn missing_value_is_an_error() {
        assert_eq!(
            parse(argv(&["-page"])),
            Err(ParseError::MissingValue("page".to_string()))
        );
    }

    #[test]
    fn a_positional_argument_stops_flag_parsing() {
        let a = parse(argv(&["schedule", "exec", "-page", "x"])).unwrap();
        assert_eq!(a, Args::default());
    }
}
