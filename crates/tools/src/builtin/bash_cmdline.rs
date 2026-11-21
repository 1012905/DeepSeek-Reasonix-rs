//! The preflight that refuses argv the OS cannot launch (parity targets:
//! `bash_cmdline.go` + `bash_cmdline_windows.go`). Windows CreateProcessW's
//! lpCommandLine ceiling is 32767 UTF-16 code units including the
//! terminating NUL; the count uses the same per-argument escaping os/exec
//! applies, so the refusal names the host limit instead of a misleading
//! "filename or extension is too long" from process creation.

#![forbid(unsafe_code)]

use crate::refusal::Refusal;

/// `shell.command_line_too_long` — a launch the OS would refuse because the
/// command line itself is over its length ceiling; nothing was started.
pub const CODE_COMMAND_LINE_TOO_LONG: &str = "shell.command_line_too_long";

/// CreateProcessW's lpCommandLine ceiling in UTF-16 code units, the
/// terminating NUL included.
#[cfg(windows)]
pub const WINDOWS_COMMAND_LINE_MAX: usize = 32767;

/// Go `syscall.EscapeArg`: the Windows command-line quoting os/exec applies
/// to every argument. Backslashes double only where quoting reads them:
/// before a `"` the scanner emits, and at the close of a quoted argument.
pub fn escape_arg(s: &str) -> String {
    if s.is_empty() {
        return String::from("\"\"");
    }
    let b = s.as_bytes();
    if !b.iter().any(|&c| matches!(c, b'"' | b'\\' | b' ' | b'\t')) {
        return s.to_owned();
    }
    let needs_backslash = b.iter().any(|&c| c == b'"' || c == b'\\');
    let has_space = b.iter().any(|&c| c == b' ' || c == b'\t');
    if !needs_backslash {
        return format!("\"{s}\"");
    }
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 2);
    if has_space {
        out.push(b'"');
    }
    let mut slashes = 0usize;
    for &c in b {
        match c {
            b'\\' => {
                slashes += 1;
                out.push(c);
            }
            b'"' => {
                out.extend(std::iter::repeat_n(b'\\', slashes + 1));
                slashes = 0;
                out.push(b'"');
            }
            _ => {
                slashes = 0;
                out.push(c);
            }
        }
    }
    if has_space {
        out.extend(std::iter::repeat_n(b'\\', slashes));
        out.push(b'"');
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The UTF-16 units CreateProcessW receives for argv, NUL included, and the
/// ceiling this OS enforces (0 means no check off Windows).
pub fn command_line_units(argv: &[String]) -> (usize, usize) {
    if !cfg!(windows) {
        return (0, 0);
    }
    let mut units = 1;
    for (i, arg) in argv.iter().enumerate() {
        if i > 0 {
            units += 1;
        }
        units += escape_arg(arg).encode_utf16().count();
    }
    (units, WINDOWS_COMMAND_LINE_MAX)
}

/// Refuses argv that cannot be launched on this OS, with the identity the
/// host reports and the remedy the model acts on.
pub fn check_command_line(argv: &[String]) -> Result<(), Refusal> {
    let (units, limit) = command_line_units(argv);
    if limit == 0 || units <= limit {
        return Ok(());
    }
    Err(Refusal::code(
        CODE_COMMAND_LINE_TOO_LONG,
        format!(
            "the command is {units} characters and Windows accepts at most {limit}, so it was not run. \
             Write long text with write_file or edit_file and have the command read that file, \
             or split the work into smaller commands"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expectations recorded from Go syscall.EscapeArg output, spelled as
    // raw strings so backslash counts read literally.
    #[test]
    fn escape_arg_matches_go_syscall() {
        assert_eq!(escape_arg(""), "\"\"");
        assert_eq!(escape_arg("plain"), "plain");
        assert_eq!(escape_arg("a b"), "\"a b\"");
        assert_eq!(escape_arg("a\"b"), "a\\\"b");
        assert_eq!(escape_arg(r"a\b"), r"a\b");
        assert_eq!(escape_arg(r#"a\"b"#), r#"a\\\"b"#);
        assert_eq!(
            escape_arg(r"C:\Program Files\x.exe arg"),
            r#""C:\Program Files\x.exe arg""#
        );
        assert_eq!(escape_arg(r"trailing\"), r"trailing\");
        assert_eq!(escape_arg("tab\tsep"), "\"tab\tsep\"");
    }

    #[test]
    fn command_line_units_count_the_escaping() {
        let argv = vec![
            String::from("bash"),
            String::from("-c"),
            String::from("echo hi"),
        ];
        let (units, limit) = command_line_units(&argv);
        if cfg!(windows) {
            // bash + sep + -c + sep + "echo hi" + NUL
            assert_eq!(
                units,
                1 + 4 + 1 + 2 + 1 + 9,
                "escaped units with separators"
            );
            assert_eq!(limit, 32767);
        } else {
            assert_eq!((units, limit), (0, 0), "no Windows ceiling off Windows");
        }
    }

    #[test]
    fn check_command_line_refuses_over_the_ceiling() {
        if !cfg!(windows) {
            return;
        }
        let big = "中".repeat(32800);
        let argv = vec![String::from("powershell"), format!("Write-Output '{big}'")];
        let err = check_command_line(&argv).unwrap_err();
        assert_eq!(err.code, CODE_COMMAND_LINE_TOO_LONG);
        assert!(err.message.contains("was not run"), "{}", err.message);
        let small = vec![
            String::from("bash"),
            String::from("-c"),
            String::from("echo ok"),
        ];
        assert!(check_command_line(&small).is_ok());
    }
}
