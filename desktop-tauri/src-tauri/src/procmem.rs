// Per-process memory rows for the kernel's diagnostics panel: a fixed kind
// and two whole numbers each. No pid, name, path or command line leaves here.
use serde::Serialize;
use std::collections::HashMap;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

const MAX_ROWS: usize = 11;
const KB_CAP: i64 = 2_i64.pow(31);

#[derive(Serialize)]
pub struct ProcRow {
    process: String,
    #[serde(rename = "workingSetKB")]
    working_set_kb: i64,
    #[serde(rename = "privateKB")]
    private_kb: i64,
}

pub fn kb(bytes: u64) -> i64 {
    let rounded = (bytes as f64 / 1024.0).round();
    if rounded.is_finite() && rounded > 0.0 {
        (rounded as i64).min(KB_CAP)
    } else {
        0
    }
}

/// The webview helpers name their kind on the command line; everything that
/// is not the shell itself or a named helper is a utility.
pub fn kind(cmd: &[std::ffi::OsString]) -> &'static str {
    for arg in cmd {
        let Some(flag) = arg.to_str().and_then(|a| a.strip_prefix("--type=")) else {
            continue;
        };
        return match flag {
            "renderer" => "renderer",
            "gpu-process" => "gpu",
            _ => "utility",
        };
    }
    "utility"
}

fn descendants(sys: &System, own: Pid) -> Vec<Pid> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, process) in sys.processes() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut out = Vec::new();
    let mut queue = vec![own];
    while let Some(pid) = queue.pop() {
        if let Some(kids) = children.get(&pid) {
            for kid in kids {
                out.push(*kid);
                queue.push(*kid);
            }
        }
    }
    out
}

pub fn rows(sys: &System, own: Pid) -> Vec<ProcRow> {
    let mut pids = vec![own];
    pids.extend(descendants(sys, own));
    pids.truncate(MAX_ROWS);
    pids.iter()
        .filter_map(|pid| {
            let process = sys.process(*pid)?;
            let name = if *pid == own {
                "browser"
            } else {
                kind(process.cmd())
            };
            Some(ProcRow {
                process: name.into(),
                working_set_kb: kb(process.memory()),
                private_kb: private_kb(*pid, process.memory()),
            })
        })
        .collect()
}

#[cfg(windows)]
fn private_kb(pid: Pid, working_set: u64) -> i64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };
    let Ok(handle) = (unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            pid.as_u32(),
        )
    }) else {
        return kb(working_set);
    };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let ok = unsafe {
        GetProcessMemoryInfo(
            handle,
            &mut counters as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    }
    .is_ok();
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    if ok {
        kb(counters.PrivateUsage as u64)
    } else {
        kb(working_set)
    }
}

// No platform here exposes private bytes through sysinfo; the working set is
// the honest answer rather than a second name for an unread number.
#[cfg(not(windows))]
fn private_kb(_pid: Pid, working_set: u64) -> i64 {
    kb(working_set)
}

#[tauri::command]
pub async fn shell_process_memory() -> Result<Vec<ProcRow>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let Ok(sys) = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
        ) else {
            return Vec::new();
        };
        let own = Pid::from_u32(std::process::id());
        rows(&sys, own)
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn kb_rounds_and_clamps() {
        assert_eq!(kb(0), 0);
        assert_eq!(kb(1024), 1);
        assert_eq!(kb(1536), 2);
        assert_eq!(kb(u64::MAX), KB_CAP);
    }

    #[test]
    fn kinds_come_from_the_type_flag() {
        let cmd = |s: &str| vec![OsString::from("webview"), OsString::from(s)];
        assert_eq!(kind(&cmd("--type=renderer")), "renderer");
        assert_eq!(kind(&cmd("--type=gpu-process")), "gpu");
        assert_eq!(kind(&cmd("--type=utility")), "utility");
        assert_eq!(kind(&cmd("--type=crashpad-handler")), "utility");
        assert_eq!(kind(&[OsString::from("shell")]), "utility");
    }

    #[test]
    fn rows_cap_at_eleven_and_name_the_shell_browser() {
        let Ok(sys) = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
        ) else {
            return;
        };
        let own = Pid::from_u32(std::process::id());
        let rows = rows(&sys, own);
        assert!(rows.len() <= MAX_ROWS);
        assert_eq!(rows[0].process, "browser");
    }
}
