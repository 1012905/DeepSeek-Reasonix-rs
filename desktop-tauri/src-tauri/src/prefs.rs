// The page's own preferences, kept past the origin that stored them. The kernel
// listens on a fresh loopback port each launch and localStorage is keyed by
// origin, so without this every launch would open on a page that remembered
// nothing. Only the page's own string keys are kept.
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "rx-";

pub type Prefs = BTreeMap<String, String>;

pub fn file(config_dir: &Path) -> PathBuf {
    config_dir.join("window-prefs.json")
}

pub fn pick(value: &Map<String, Value>) -> Prefs {
    value
        .iter()
        .filter(|(key, _)| key.starts_with(PREFIX))
        .filter_map(|(key, v)| v.as_str().map(|s| (key.clone(), s.to_string())))
        .collect()
}

pub fn load(file: &Path) -> Prefs {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Map<String, Value>>(&text).ok())
        .map(|map| pick(&map))
        .unwrap_or_default()
}

/// Written beside and renamed over, so a crash mid-write leaves the last good
/// copy rather than half of one.
pub fn save(file: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(prefs)?)?;
    std::fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_keeps_only_page_strings() {
        let value: Map<String, Value> =
            serde_json::from_str(r#"{"rx-theme":"dark","rx-n":3,"other":"x","rx-lang":"zh"}"#)
                .expect("json");
        let picked = pick(&value);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked["rx-theme"], "dark");
        assert_eq!(picked["rx-lang"], "zh");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = crate::scratch("prefs");
        let path = file(&dir);
        let mut prefs = Prefs::new();
        prefs.insert("rx-theme".into(), "light".into());
        save(&path, &prefs).expect("saves");
        assert_eq!(load(&path), prefs);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_broken_file_loads_empty() {
        let dir = crate::scratch("prefs-broken");
        assert!(load(&dir.join("absent.json")).is_empty());
        std::fs::write(dir.join("broken.json"), "{not json").expect("write");
        assert!(load(&dir.join("broken.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
