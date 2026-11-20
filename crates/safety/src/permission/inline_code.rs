//! Which argv shapes run a program written in the call itself — `python -c`,
//! `node -e`, `bash -c`. Permission and evidence must agree on it, so the one
//! table serves both.

/// Reports whether an argv runs a program written in the call itself —
/// `python -c`, `node -e`, `bash -c`, `deno eval`.
pub fn argv_carries_inline_code(argv: &[String]) -> bool {
    let Some(first) = argv.first() else {
        return false;
    };
    let Some(check) = inline_code_flags_for(&executable_base(first)) else {
        return false;
    };
    check(&argv[1..])
}

/// Reports whether a program runs source handed to it rather than a fixed
/// operation. It answers the same table as [`argv_carries_inline_code`], for
/// callers holding a fact other than a flag — a here-doc on the interpreter's
/// stdin is `-c` by another spelling.
pub fn is_inline_code_interpreter(program: &str) -> bool {
    inline_code_flags_for(&executable_base(program)).is_some()
}

/// Strips directory and .exe so a path and a bare name classify the same.
pub fn executable_base(command: &str) -> String {
    let mut command = command;
    if let Some(i) = command.rfind(['/', '\\']) {
        command = &command[i + 1..];
    }
    command
        .strip_suffix(".exe")
        .unwrap_or(command)
        .to_ascii_lowercase()
}

/// The one table: which programs take handed-in source, and how each spells
/// the flag that carries it. Membership and spelling come from the same place
/// so a program cannot be known to one caller and unknown to the next.
fn inline_code_flags_for(base: &str) -> Option<fn(&[String]) -> bool> {
    match base {
        "bash" | "dash" | "fish" | "ksh" | "sh" | "zsh" => Some(has_shell_command_flag),
        "powershell" | "pwsh" => Some(flags_of(&[
            "-c",
            "-command",
            "-e",
            "-enc",
            "-encodedcommand",
        ])),
        "cmd" => Some(flags_of(&["/c", "/k"])),
        "node" | "bun" => Some(flags_of(&["-e", "--eval", "-p", "--print"])),
        "deno" => Some(flags_of(&["eval"])),
        "python" | "python3" | "py" | "pypy" | "pypy3" => Some(flags_of(&["-c"])),
        "perl" | "ruby" | "lua" | "luajit" | "r" | "rscript" | "osascript" => {
            Some(flags_of(&["-e"]))
        }
        "php" => Some(flags_of(&["-r"])),
        "awk" | "gawk" | "mawk" | "nawk" => Some(awk_carries_program),
        _ => None,
    }
}

/// awk takes its program as the first operand, so a call carries inline code
/// unless a program file is named with -f or --file.
fn awk_carries_program(args: &[String]) -> bool {
    let mut file = false;
    for arg in args {
        if arg == "--" {
            return !file;
        }
        if arg == "-e" || arg == "--source" || arg.starts_with("--source=") {
            return true;
        }
        let long_file = arg == "--file" || arg.starts_with("--file=");
        let short_file = arg == "-f" || (arg.starts_with("-f") && !arg.starts_with("--"));
        if long_file || short_file {
            file = true;
        }
    }
    !file
}

fn flags_of(candidates: &'static [&'static str]) -> fn(&[String]) -> bool {
    match candidates {
        SHELL_PS => ps_flags,
        CMD => cmd_flags,
        NODE => node_flags,
        DENO => deno_flags,
        PYTHON => python_flags,
        PERL => perl_flags,
        PHP => php_flags,
        _ => unreachable!(),
    }
}

const SHELL_PS: &[&str] = &["-c", "-command", "-e", "-enc", "-encodedcommand"];
const CMD: &[&str] = &["/c", "/k"];
const NODE: &[&str] = &["-e", "--eval", "-p", "--print"];
const DENO: &[&str] = &["eval"];
const PYTHON: &[&str] = &["-c"];
const PERL: &[&str] = &["-e"];
const PHP: &[&str] = &["-r"];

fn flags(args: &[String], candidates: &[&str]) -> bool {
    args.iter().any(|arg| has_inline_flag(arg, candidates))
}
fn ps_flags(args: &[String]) -> bool {
    flags(args, SHELL_PS)
}
fn cmd_flags(args: &[String]) -> bool {
    flags(args, CMD)
}
fn node_flags(args: &[String]) -> bool {
    flags(args, NODE)
}
fn deno_flags(args: &[String]) -> bool {
    flags(args, DENO)
}
fn python_flags(args: &[String]) -> bool {
    flags(args, PYTHON)
}
fn perl_flags(args: &[String]) -> bool {
    flags(args, PERL)
}
fn php_flags(args: &[String]) -> bool {
    flags(args, PHP)
}

/// Reports a POSIX shell's -c, including bundled forms such as `sh -lc`. A
/// bare `--` ends option parsing, so nothing after it counts.
fn has_shell_command_flag(args: &[String]) -> bool {
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        if lower == "--" {
            return false;
        }
        if lower == "--command" {
            return true;
        }
        if lower.starts_with('-') && !lower.starts_with("--") && lower[1..].contains('c') {
            return true;
        }
    }
    false
}

fn has_inline_flag(arg: &str, candidates: &[&str]) -> bool {
    let lower = arg.to_ascii_lowercase();
    candidates.iter().any(|c| inline_flag_matches(&lower, c))
}

/// Accepts the spellings a flag actually takes: exact, an attached value
/// (--eval=…, -enc:…), and a bundled short form (-pe).
fn inline_flag_matches(arg: &str, candidate: &str) -> bool {
    if arg == candidate {
        return true;
    }
    if candidate.starts_with("--") {
        return arg.starts_with(&format!("{candidate}="))
            || arg.starts_with(&format!("{candidate}:"));
    }
    if candidate.starts_with('/') {
        return candidate.len() == 2 && arg.starts_with(candidate);
    }
    if candidate.starts_with('-') && candidate.len() == 2 {
        return arg.starts_with(candidate) && !arg.starts_with("--");
    }
    if candidate.starts_with('-') {
        return arg.starts_with(&format!("{candidate}:"));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn inline_code_table() {
        assert!(argv_carries_inline_code(&argv(&[
            "python3", "-c", "print(1)"
        ])));
        assert!(argv_carries_inline_code(&argv(&["node", "-e", "x"])));
        assert!(argv_carries_inline_code(&argv(&["node", "--eval=x"])));
        assert!(argv_carries_inline_code(&argv(&["bash", "-lc", "x"])));
        assert!(argv_carries_inline_code(&argv(&["pwsh", "-Command", "x"])));
        assert!(argv_carries_inline_code(&argv(&["cmd", "/cecho", "x"])));
        assert!(argv_carries_inline_code(&argv(&[
            "awk",
            "BEGIN{system(\"id\")}"
        ])));
        assert!(!argv_carries_inline_code(&argv(&[
            "awk",
            "-f",
            "report.awk",
            "data.txt"
        ])));
        assert!(!argv_carries_inline_code(&argv(&["git", "status"])));
        assert!(!argv_carries_inline_code(&argv(&[])));
        assert!(argv_carries_inline_code(&argv(&[
            "/usr/bin/python3",
            "-c",
            "x"
        ])));
    }

    #[test]
    fn interpreter_names() {
        assert!(is_inline_code_interpreter("python"));
        assert!(is_inline_code_interpreter("bash.exe"));
        assert!(!is_inline_code_interpreter("ls"));
        assert_eq!(executable_base("C:/tools/Rm.exe"), "rm");
        assert_eq!(executable_base("/bin/sh"), "sh");
    }
}
