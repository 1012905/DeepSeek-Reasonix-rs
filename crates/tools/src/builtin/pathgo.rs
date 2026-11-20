//! Go `path/filepath` semantics the confinement checks depend on: lexical
//! cleaning, relative paths, join, and volume handling. Parity target:
//! go/src/path/filepath (Clean, Rel, Join, Abs, Split, IsAbs).
//!
//! All comparisons are byte-wise like Go; UNC volume lengths follow the
//! filepathlite shape closely enough for the confinement paths the tools
//! produce.

#![forbid(unsafe_code)]

pub const PATH_SEPARATORS_UNIX: &str = "/";
pub const PATH_SEPARATORS_WINDOWS: &str = "/\\";

#[inline]
pub fn is_path_separator(b: u8) -> bool {
    if cfg!(windows) {
        b == b'/' || b == b'\\'
    } else {
        b == b'/'
    }
}

/// The platform separator as a byte (Go `filepath.Separator`).
#[inline]
pub fn separator() -> char {
    if cfg!(windows) { '\\' } else { '/' }
}

/// Go `filepath.VolumeNameLen`: drive letter or UNC prefix length. POSIX
/// names carry no volume — the Windows forms below are compile-time dead
/// there, which is what makes `clean("//a//b")` answer `/a/b` on unix like
/// Go's unix Clean does.
pub fn volume_name_len(path: &str) -> usize {
    if cfg!(unix) {
        return 0;
    }
    let b = path.as_bytes();
    if b.len() >= 2 && is_path_separator(b[0]) && is_path_separator(b[1]) {
        // UNC: two leading separators, then host\share; the volume ends at
        // the separator closing the share (filepathlite uncLen).
        let n = b.len();
        let mut i = 2;
        while i < n && !is_path_separator(b[i]) {
            i += 1;
        }
        if i == n {
            return n;
        }
        i += 1;
        while i < n && !is_path_separator(b[i]) {
            i += 1;
        }
        return i;
    }
    if b.len() >= 2 && b[1] == b':' && (b[0] | 0x20) >= b'a' && (b[0] | 0x20) <= b'z' {
        return 2;
    }
    0
}

/// Go `filepath.VolumeName`.
pub fn volume_name(path: &str) -> String {
    let n = volume_name_len(path);
    String::from(&path[..n])
}

/// Go `filepath.Clean` — purely lexical, separator-aware, volume-preserving.
pub fn clean(path: &str) -> String {
    let original = path.as_bytes();
    let vol_len = volume_name_len(path);
    let rest = &original[vol_len..];
    if rest.is_empty() {
        if vol_len > 1 && is_path_separator(original[0]) {
            return from_slash(path);
        }
        return format!("{path}.");
    }
    let rooted = is_path_separator(rest[0]);

    // Go lazybuf counts w over the appended body only; the volume prefix is
    // spliced in at string time. Every offset below is body-relative.
    let mut out: Vec<u8> = Vec::with_capacity(rest.len());
    let mut w = 0usize;
    let mut dotdot = 0usize;
    let mut r = 0usize;
    if rooted {
        out.push(separator() as u8);
        w = 1;
        r = 1;
        dotdot = 1;
    }
    let n = rest.len();
    while r < n {
        if is_path_separator(rest[r])
            || (rest[r] == b'.' && (r + 1 == n || is_path_separator(rest[r + 1])))
        {
            r += 1;
        } else if rest[r] == b'.'
            && r + 1 < n
            && rest[r + 1] == b'.'
            && (r + 2 == n || is_path_separator(rest[r + 2]))
        {
            r += 2;
            if w > dotdot {
                w -= 1;
                while w > dotdot && !is_path_separator(out[w]) {
                    w -= 1;
                }
                out.truncate(w);
            } else if !rooted {
                if w > 0 {
                    out.push(separator() as u8);
                    w += 1;
                }
                out.extend_from_slice(b"..");
                w += 2;
                dotdot = w;
            }
        } else {
            if (rooted && w != 1) || (!rooted && w != 0) {
                out.push(separator() as u8);
                w += 1;
            }
            while r < n && !is_path_separator(rest[r]) {
                out.push(rest[r]);
                w += 1;
                r += 1;
            }
        }
    }
    if w == 0 {
        out.push(b'.');
    }
    from_slash(&format!(
        "{}{}",
        String::from_utf8_lossy(&original[..vol_len]),
        String::from_utf8_lossy(&out)
    ))
}

/// Go `filepath.IsAbs`.
pub fn is_abs(path: &str) -> bool {
    let vol = volume_name_len(path);
    if vol == 0 {
        // On POSIX a leading separator roots the path; on Windows a path
        // without a volume is rootless (drive-relative).
        return cfg!(unix)
            && path
                .as_bytes()
                .first()
                .is_some_and(|&b| is_path_separator(b));
    }
    if vol == 2 && path.as_bytes()[1] == b':' {
        let b = path.as_bytes();
        return b.len() > 2 && is_path_separator(b[2]);
    }
    true
}

/// Go `filepath.Join`: empty elements dropped, result cleaned; no elements
/// at all yields the empty string.
pub fn join(parts: &[&str]) -> String {
    let non_empty: Vec<&str> = parts.iter().filter(|p| !p.is_empty()).copied().collect();
    if non_empty.is_empty() {
        return String::new();
    }
    let sep = separator();
    clean(&non_empty.join(&sep.to_string()))
}

/// Go `filepath.Abs` — absolute paths are cleaned, relative ones joined onto
/// the process working directory.
pub fn abs_path(path: &str) -> std::io::Result<String> {
    if is_abs(path) {
        return Ok(clean(path));
    }
    let wd = std::env::current_dir()?;
    let wd = wd.to_string_lossy().into_owned();
    Ok(join(&[&wd, path]))
}

/// Go `filepath.Split`.
pub fn split(path: &str) -> (String, String) {
    let b = path.as_bytes();
    let vol = volume_name_len(path);
    let mut i = b.len() as isize - 1;
    while i >= vol as isize && !is_path_separator(b[i as usize]) {
        i -= 1;
    }
    let cut = (i + 1).max(0) as usize;
    (String::from(&path[..cut]), String::from(&path[cut..]))
}

/// Go `filepath.Rel`, component-wise; the error mirrors Go's Rel failure
/// when the two sides cannot be made relative (volume mismatch or one side
/// rooted and the other not). Volume and path comparisons fold case on
/// case-insensitive platforms, like Go's sameWord.
/// The error Go's filepath.Rel returns when the two sides cannot be made
/// relative (volume mismatch or one side rooted and the other not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Rel: cannot make the paths relative")]
pub struct RelError;

pub fn rel(basepath: &str, targpath: &str) -> Result<String, RelError> {
    let fold = cfg!(windows) || cfg!(target_os = "macos");
    let same_word = |a: &str, b: &str| fold && a.eq_ignore_ascii_case(b) || a == b;
    let base_vol = volume_name(basepath);
    let targ_vol = volume_name(targpath);
    let mut base = clean(basepath);
    let mut targ = clean(targpath);
    let vol_same = same_word(&targ_vol, &base_vol);
    if vol_same {
        base = base[base_vol.len()..].to_owned();
        targ = targ[targ_vol.len()..].to_owned();
    }
    if same_word(&targ, &base) {
        return Ok(String::from("."));
    }
    if base == "." {
        base = String::new();
    }
    let b = base.as_bytes();
    let t = targ.as_bytes();
    let base_slashed = !b.is_empty() && is_path_separator(b[0]);
    let targ_slashed = !t.is_empty() && is_path_separator(t[0]);
    if base_slashed != targ_slashed || !vol_same {
        return Err(RelError);
    }
    let base_parts = components(&base, base_slashed);
    let targ_parts = components(&targ, targ_slashed);
    let mut k = 0usize;
    while k < base_parts.len() && k < targ_parts.len() && same_word(base_parts[k], targ_parts[k]) {
        k += 1;
    }
    let ups = base_parts.len() - k;
    let tail = targ_parts[k..].join(&separator().to_string());
    let mut out = String::new();
    for i in 0..ups {
        if i > 0 {
            out.push(separator());
        }
        out.push_str("..");
    }
    if !tail.is_empty() {
        if !out.is_empty() {
            out.push(separator());
        }
        out.push_str(&tail);
    }
    if out.is_empty() {
        return Ok(String::from("."));
    }
    Ok(out)
}

fn components(path: &str, rooted: bool) -> Vec<&str> {
    let b = path.as_bytes();
    let start = if rooted && !b.is_empty() { 1 } else { 0 };
    let body = &path[start..];
    if body.is_empty() {
        return Vec::new();
    }
    body.split(separator()).filter(|c| !c.is_empty()).collect()
}

/// Go `filepath.ToSlash` / `FromSlash`.
pub fn to_slash(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        String::from(path)
    }
}

pub fn from_slash(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        String::from(path)
    }
}

/// Go `filepath.Glob`'s hasMeta.
pub fn has_meta(path: &str) -> bool {
    let magic = if cfg!(windows) { "*?[" } else { "*?[\\" };
    path.chars().any(|c| magic.contains(c))
}

/// Renders a duration the way Go's time.Duration.String does for the
/// whole-second values the tools produce ("30s", "1m30s", "2m0s").
pub fn go_duration_string(secs: u64) -> String {
    if secs == 0 {
        return String::from("0s");
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if m > 0 || h > 0 {
        out.push_str(&format!("{m}m"));
    }
    out.push_str(&format!("{s}s"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_matches_go_semantics() {
        if cfg!(windows) {
            assert_eq!(clean("C:\\work\\proj"), "C:\\work\\proj");
            assert_eq!(clean("C:\\work\\proj\\..\\proj\\x"), "C:\\work\\proj\\x");
            assert_eq!(clean("C:\\work\\proj\\.."), "C:\\work");
            assert_eq!(clean("C:\\"), "C:\\");
            assert_eq!(clean("a//b\\\\c\\\\."), "a\\b\\c");
            assert_eq!(clean(""), ".");
        } else {
            assert_eq!(clean("/work/proj"), "/work/proj");
            assert_eq!(clean("/work/proj/../proj/x"), "/work/proj/x");
            assert_eq!(clean("//a//b"), "/a/b");
            assert_eq!(clean(""), ".");
        }
    }

    #[test]
    fn rel_walks_up_and_down() {
        let sep = separator();
        let (root, other, up) = if cfg!(windows) {
            ("C:\\a\\b", "C:\\a\\c", "C:\\a\\x")
        } else {
            ("/a/b", "/a/c", "/a/x")
        };
        assert_eq!(rel(root, other).unwrap(), format!("..{sep}c"));
        assert_eq!(rel(root, root).unwrap(), ".");
        assert_eq!(rel(root, up).unwrap(), format!("..{sep}x"));
        let parent = if cfg!(windows) { "C:\\a" } else { "/a" };
        assert_eq!(rel(root, parent).unwrap(), "..");
        assert_eq!(rel(parent, root).unwrap(), "b");
    }

    #[test]
    fn join_and_split() {
        assert_eq!(
            join(&["a", "b"]),
            if cfg!(windows) { "a\\b" } else { "a/b" }
        );
        assert_eq!(
            join(&["", "**", "x.go"]),
            if cfg!(windows) { "**\\x.go" } else { "**/x.go" }
        );
        let (dir, file) = split(if cfg!(windows) {
            "src\\*.go"
        } else {
            "src/*.go"
        });
        assert_eq!(file, "*.go");
        assert_eq!(dir, if cfg!(windows) { "src\\" } else { "src/" });
    }

    #[test]
    fn duration_format_is_go_style() {
        assert_eq!(go_duration_string(30), "30s");
        assert_eq!(go_duration_string(90), "1m30s");
        assert_eq!(go_duration_string(300), "5m0s");
    }
}
