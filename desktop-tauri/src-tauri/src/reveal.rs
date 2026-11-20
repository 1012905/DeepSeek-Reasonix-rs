// Showing a workspace entry in the platform's file manager. The page names a
// pane and a workspace-relative path, never a location on disk: the kernel owns
// the root and refuses whatever resolves outside it, and only its answer is
// shown. Nothing here is opened unless it is plainly a folder or a document —
// a directory can be an application bundle, and opening one launches it.
use crate::client::KernelClient;
use serde::{Deserialize, Serialize};
use std::path::Path;

const DOCUMENT_MEDIA: &[&str] = &[
    "pdf", "txt", "md", "csv", "tsv", "json", "yaml", "yml", "xml", "rtf", "docx", "xlsx", "pptx",
    "odt", "ods", "odp", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic",
    "mp3", "wav", "ogg", "m4a", "flac", "mp4", "mov", "webm", "mkv",
];

/// A kernel refusal as the page reads it: the code, and English fallback.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Refusal {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
}

pub fn refused(error: impl Into<String>) -> Refusal {
    Refusal {
        error: error.into(),
        ..Default::default()
    }
}

/// A pane's base as the hub publishes it. Nothing else may steer a request this
/// process sends with the credential, and the unprefixed root is no pane.
fn pane_base(base: &str) -> bool {
    base.strip_prefix("/rt/").is_some_and(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// What this machine accepts as a place to show. On Windows a share or a device
/// namespace is reached just by being selected, so only a drive-letter path passes.
pub fn local(target: &str, windows: bool) -> bool {
    if !windows {
        return target.starts_with('/');
    }
    let b = target.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

fn ask(client: &KernelClient, path_and_query: &str, windows: bool) -> Result<String, Refusal> {
    let answer = client.get(path_and_query).map_err(refused)?;
    let body: Option<serde_json::Value> = serde_json::from_str(&answer.body).ok();
    if !(200..300).contains(&answer.status) {
        let coded = body
            .filter(|b| b.get("code").is_some_and(|c| c.is_string()))
            .and_then(|b| serde_json::from_value::<Refusal>(b).ok());
        return Err(coded.unwrap_or_else(|| refused(format!("HTTP {}", answer.status))));
    }
    match body
        .as_ref()
        .and_then(|b| b.get("path"))
        .and_then(|p| p.as_str())
    {
        Some(path) if local(path, windows) => Ok(path.to_string()),
        _ => Err(refused("the kernel named no location on this machine")),
    }
}

fn with_query(path: &str, key: &str, value: &str) -> String {
    let mut url: tauri::Url = format!("http://kernel{path}")
        .parse()
        .expect("a fixed base parses");
    url.query_pairs_mut().append_pair(key, value);
    format!("{}?{}", url.path(), url.query().unwrap_or(""))
}

pub fn locate_pane(
    client: &KernelClient,
    base: &str,
    rel: &str,
    windows: bool,
) -> Result<String, Refusal> {
    if !pane_base(base) {
        return Err(refused("not a pane this window publishes"));
    }
    let path = format!("{base}/workspace/locate");
    let path = if rel.is_empty() {
        path
    } else {
        with_query(&path, "path", rel)
    };
    ask(client, &path, windows)
}

/// A project the sidebar lists needs no pane: the hub answers for the folders
/// it lists and refuses any other.
pub fn locate_workspace(
    client: &KernelClient,
    root: &str,
    windows: bool,
) -> Result<String, Refusal> {
    ask(
        client,
        &with_query("/host/workspaces/locate", "root", root),
        windows,
    )
}

/// A folder the opener may enter: a directory, and on macOS not an application
/// bundle, which opening would launch instead of showing.
pub fn plain_folder(target: &Path) -> bool {
    if !target.is_dir() {
        return false;
    }
    !(cfg!(target_os = "macos") && target.join("Contents").join("Info.plist").exists())
}

pub fn openable_document(target: &Path) -> Result<(), Refusal> {
    if !target.is_file() {
        return Err(refused("not a regular file"));
    }
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if DOCUMENT_MEDIA.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(refused("only documents and media can be opened"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_published_pane_bases_steer_a_request() {
        assert!(pane_base("/rt/abc_D-1"));
        assert!(!pane_base("/rt/"));
        assert!(!pane_base("/rt/a/b"));
        assert!(!pane_base("/rt/../host"));
        assert!(!pane_base(""));
        assert!(!pane_base("/host"));
    }

    #[test]
    fn local_admits_drive_paths_on_windows_and_absolute_paths_elsewhere() {
        assert!(local("C:\\work\\x", true));
        assert!(local("d:/work", true));
        assert!(!local("\\\\host\\share", true));
        assert!(!local("\\\\?\\C:\\x", true));
        assert!(!local("relative", true));
        assert!(local("/home/me", false));
        assert!(!local("home/me", false));
    }

    #[test]
    fn queries_are_encoded() {
        assert_eq!(
            with_query("/rt/a/workspace/locate", "path", "a b/&c"),
            "/rt/a/workspace/locate?path=a+b%2F%26c"
        );
    }

    #[test]
    fn documents_are_told_apart_from_programs() {
        let dir = crate::scratch("reveal");
        let doc = dir.join("notes.MD");
        let program = dir.join("setup.exe");
        std::fs::write(&doc, "x").expect("doc");
        std::fs::write(&program, "x").expect("program");
        assert!(openable_document(&doc).is_ok());
        assert!(openable_document(&program).is_err());
        assert!(openable_document(&dir).is_err());
        assert!(plain_folder(&dir));
        assert!(!plain_folder(&doc));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
