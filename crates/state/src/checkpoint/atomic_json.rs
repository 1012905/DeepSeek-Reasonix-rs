use std::fs;
use std::path::Path;

use reasonix_contract::gojson::escape_html;
use reasonix_core::fileutil::{atomic_write_file_strict, mkdir_all_mode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::secure_path::SecureError;

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), SecureError> {
    let json = serde_json::to_string_pretty(value).map_err(|source| SecureError::JsonEncode {
        path: path.to_path_buf(),
        source,
    })?;
    let json = escape_html(&json);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    mkdir_all_mode(parent, 0o755)
        .map_err(|source| SecureError::io("create JSON directory", path, source))?;
    atomic_write_file_strict(path, json.as_bytes(), 0o644)
        .map_err(|source| SecureError::io("write JSON", path, source))
}

pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, SecureError> {
    let bytes = fs::read(path).map_err(|source| SecureError::JsonRead {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| SecureError::JsonDecode {
        path: path.to_path_buf(),
        source,
    })
}
