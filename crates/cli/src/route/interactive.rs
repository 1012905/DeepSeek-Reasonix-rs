//! `run` and the TUI: the decidable pre-handler checks. Session, lease, boot,
//! and completion-classification outcomes belong to the unmigrated body.

use crate::approval::{approval_conflict, yolo_active};
use crate::flags::{normalize_optional_resume_arg, parse, run_specs, tui_specs};
use crate::permission::parse_permission_mode;
use crate::preset::{self, TokenMode};
use crate::route::{CliEnv, CliOutcome, Route};
use crate::text::*;

/// Resolves `--profile` (deprecated alias) over `--preset`, returning the
/// token mode and whether the deprecation warning was triggered.
fn resolve_preset(
    profile_raw: Option<&str>,
    preset_raw: Option<&str>,
) -> Result<(TokenMode, bool), String> {
    let profile = profile_raw.unwrap_or("");
    let preset = preset_raw.unwrap_or("");
    let deprecated = !profile.is_empty();
    let raw = if deprecated { profile } else { preset };
    preset::parse_runtime_profile(raw).map(|mode| (mode, deprecated))
}

/// `reasonix run [flags] <task>`.
pub(crate) fn run_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let flags = parse(&run_specs(), rest, true);
    let route = Route::Run {
        args: rest.to_vec(),
        flags: flags.clone(),
    };
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }

    if let Some(conflict) = approval_conflict(&flags) {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {}\n", conflict.message()),
            2,
            route,
        );
    }

    let output_format = flags.value("output-format").unwrap_or("text");
    if flags.changed("output-format") && !matches!(output_format, "text" | "json" | "stream-json") {
        return CliOutcome::new(
            String::new(),
            format!(
                "{ERROR_PREFIX} unknown output format {:?} (want text, json, or stream-json)\n",
                output_format
            ),
            2,
            route,
        );
    }

    if flags.bool("events-jsonl") && flags.changed("output-format") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {EVENTS_JSONL_CONFLICT}\n"),
            2,
            route,
        );
    }

    let mut stderr = String::new();
    match resolve_preset(flags.value("profile"), flags.value("preset")) {
        Ok((_, deprecated)) => {
            if deprecated {
                stderr.push_str(PROFILE_DEPRECATED_WARNING);
                stderr.push('\n');
            }
        }
        Err(err) => {
            return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
        }
    }

    let mode_raw = flags.value("permission-mode").unwrap_or("");
    let mode = match parse_permission_mode(mode_raw) {
        Ok(mode) => mode,
        Err(err) => {
            return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
        }
    };
    if mode.plan {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {PLAN_REQUIRES_INTERACTIVE}\n"),
            2,
            route,
        );
    }

    // `--copy` needs a session to copy from (CLI-024); a fresh `--continue`
    // start skips the copy, so the check only fires with neither resume nor
    // continue set.
    if flags.bool("copy") && flags.value("resume").is_none() && !flags.bool("continue") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {COPY_REQUIRES_RESUME}\n"),
            2,
            route,
        );
    }

    if flags.positionals.is_empty() && env.stdin_text.trim().is_empty() {
        return CliOutcome::new(String::new(), format!("{USAGE_RUN_HINT}\n"), 2, route);
    }

    stderr.push_str(&crate::route::unmigrated_notice("run"));
    CliOutcome::new(String::new(), stderr, 2, route)
}

/// `reasonix tui|chat|code [flags]`. Flag parsing and the non-terminal guard
/// are deterministic; the interactive session itself is the unmigrated body.
pub(crate) fn tui_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let normalized = normalize_optional_resume_arg(rest);
    let flags = parse(&tui_specs(), &normalized, true);
    let route = Route::Tui {
        args: normalized.clone(),
        flags: flags.clone(),
    };
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }

    // The TUI has no `--auto`; the yolo/permission-mode conflict still applies.
    let yolo = yolo_active(&flags);
    let mode_explicit = flags.changed("permission-mode");
    if yolo && mode_explicit {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {YOLO_PERMISSION_CONFLICT}\n"),
            2,
            route,
        );
    }

    let mut tui_stderr = String::new();
    match resolve_preset(flags.value("profile"), flags.value("preset")) {
        Ok((_, deprecated)) => {
            if deprecated {
                tui_stderr.push_str(PROFILE_DEPRECATED_WARNING);
                tui_stderr.push('\n');
            }
        }
        Err(err) => {
            return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
        }
    }

    // TUI copy without a resume target reports through the resume resolver and
    // exits 1 (CLI-024, source: tui_command.go:223-227).
    if flags.bool("copy") && flags.value("resume").is_none() && !flags.bool("continue") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {COPY_REQUIRES_RESUME}\n"),
            1,
            route,
        );
    }

    if !(env.stdin_is_tty && env.stdout_is_tty) {
        if env.mintty {
            tui_stderr.push_str(MINTTY_CONSOLE_HINT);
            return CliOutcome::new(String::new(), tui_stderr, 2, route);
        }
        tui_stderr.push_str(&format!("{ERROR_PREFIX} {TUI_NEEDS_TERMINAL}\n"));
        return CliOutcome::new(String::new(), tui_stderr, 2, route);
    }
    let mut stderr = crate::route::unmigrated_notice("tui");
    stderr.push_str(&tui_stderr);
    CliOutcome::new(String::new(), stderr, 2, route)
}
