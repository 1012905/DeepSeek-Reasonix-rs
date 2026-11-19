//! Launch workspace selection (docs/contracts/host.md §6.1): a chosen
//! directory, then a remembered project, then the auto workspace — created
//! last — never the install tree or the home directory by inheritance.

use std::path::{Path, PathBuf};

pub fn pick_launch_workspace(
    cwd: &Path,
    app_exe: &Path,
    remembered: &[PathBuf],
    auto: &Path,
) -> PathBuf {
    if user_chose_dir(cwd, app_exe) {
        return cwd.to_path_buf();
    }
    for dir in remembered {
        if dir.is_dir() {
            return dir.clone();
        }
    }
    if !auto.as_os_str().is_empty() && std::fs::create_dir_all(auto).is_ok() {
        return auto.to_path_buf();
    }
    cwd.to_path_buf()
}

/// The cwd counts only when someone chose it: a launcher inherits the install
/// directory or the filesystem root, and neither is a project.
fn user_chose_dir(cwd: &Path, app_exe: &Path) -> bool {
    if cwd.as_os_str().is_empty() || cwd.parent().is_none() {
        return false;
    }
    let install = match app_exe.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        // No stated application: an unpackaged launch keeps its cwd.
        _ => return true,
    };
    // Go's inside rule reduces to "the relative path does not climb out with
    // ..": "." (and the equal-path empty case) and subdirectories are inside;
    // anything climbing out — including exactly ".." — was chosen.
    let inside = match relative_to(install, cwd) {
        // Different volume (or otherwise unrelated): nothing ties them together,
        // so the cwd was chosen.
        None => false,
        Some(rel) => !rel.starts_with(".."),
    };
    !inside
}

/// Lexical `filepath.Rel`: `Ok` when `target` sits under `base` (or equals it),
/// `Err` when no relative path exists.
fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    let mut base = base.components().peekable();
    let mut target = target.components().peekable();
    // Windows paths compare case-insensitively for the shared prefix.
    while let (Some(b), Some(t)) = (base.peek(), target.peek()) {
        if !same_component(b.as_os_str(), t.as_os_str()) {
            break;
        }
        base.next();
        target.next();
    }
    if base.peek().is_some() {
        // `target` is not under `base`.
        return None;
    }
    let mut rel = PathBuf::new();
    for c in target {
        rel.push(c.as_os_str());
    }
    Some(rel)
}

fn same_component(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn the_install_directory_is_not_a_chosen_workspace() {
        let install = if cfg!(windows) { "C:\\apps\\studio\\bin" } else { "/apps/studio/bin" };
        let cwd = Path::new(install).parent().unwrap();
        assert!(!user_chose_dir(cwd, Path::new(install)));
    }

    #[test]
    fn the_filesystem_root_is_never_a_chosen_workspace() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(!user_chose_dir(Path::new(root), Path::new("C:\\apps\\studio\\bin")));
    }

    #[test]
    fn an_unrelated_cwd_counts_as_chosen() {
        let cwd = if cfg!(windows) { "C:\\work\\proj" } else { "/work/proj" };
        let exe = if cfg!(windows) { "C:\\apps\\studio\\bin\\host.exe" } else { "/apps/studio/bin/host" };
        assert!(user_chose_dir(Path::new(cwd), Path::new(exe)));
    }

    #[test]
    fn fallback_order_is_remembered_then_auto_then_cwd() {
        // The cwd sits in the install tree (launcher-inherited), so it is not
        // "chosen" and the fallback applies. Directories live under target/
        // because the system Temp is not writable under this environment's
        // antivirus policy.
        let cwd = p("/bin");
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/launchroot");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let auto_dir = base.join("auto");
        let got = pick_launch_workspace(&cwd, &p("/bin/host"), &[p("/missing/dir"), real.clone()], &auto_dir);
        assert_eq!(got, real);
        let got = pick_launch_workspace(&cwd, &p("/bin/host"), &[p("/missing/dir")], &auto_dir);
        assert_eq!(got, auto_dir);
        assert!(auto_dir.is_dir());
    }

    #[test]
    fn with_nothing_at_all_the_cwd_wins() {
        let cwd = p("/bin");
        let got = pick_launch_workspace(&cwd, &p("/bin/host"), &[], &p("/nowhere/auto/deep"));
        assert_eq!(got, cwd);
    }
}
