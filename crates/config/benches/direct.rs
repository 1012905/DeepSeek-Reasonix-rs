//! The shipped strategy, mirrored for the benchmark: every layer decodes
//! into an all-Optional delta and applies field-wise — the direct
//! translation of the reference decoding each file onto the accumulated
//! config. Kept beside the Value-merge benchmark so reruns always compare
//! both strategies (see benches/load.rs and src/load.rs).

use std::collections::BTreeMap;

use reasonix_config::{AuthMode, Config};
use serde::Deserialize;

/// The pure decode half of Config::load, from pre-read TOML text: the
/// strategy benchmark without filesystem noise.
pub fn decode_direct_layers(texts: &[&str]) -> Config {
    let mut config = Config::builtin_defaults();
    for (index, text) in texts.iter().enumerate() {
        let delta: LayerDelta = toml::from_str(text).expect("bench fixture parses");
        apply(&mut config, delta, index > 0);
    }
    config
}

/// One file's declared keys; every field optional so "declared" and
/// "built-in default" stay distinct.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayerDelta {
    language: Option<String>,
    serve: Option<PartialServe>,
    desktop: Option<PartialDesktop>,
    storage: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PartialServe {
    auth_mode: Option<String>,
    token: Option<String>,
    password_hash: Option<String>,
    behind_proxy: Option<bool>,
    share_port: Option<i64>,
    remember_paired_devices: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PartialDesktop {
    telemetry: Option<bool>,
    metrics: Option<bool>,
    tray: Option<String>,
}

fn apply(config: &mut Config, delta: LayerDelta, project_scope: bool) {
    if let Some(language) = delta.language {
        config.language = language;
    }
    if !project_scope {
        if let Some(serve) = delta.serve {
            if let Some(raw) = serve.auth_mode {
                config.serve.auth_mode = AuthMode::normalize(&raw);
            }
            if let Some(value) = serve.token {
                config.serve.token = value;
            }
            if let Some(value) = serve.password_hash {
                config.serve.password_hash = value;
            }
            if let Some(value) = serve.behind_proxy {
                config.serve.behind_proxy = value;
            }
            if let Some(value) = serve.share_port {
                config.serve.share_port = value;
            }
            if let Some(value) = serve.remember_paired_devices {
                config.serve.remember_paired_devices = value;
            }
        }
        if let Some(storage) = delta.storage {
            config.storage = storage;
        }
    }
    if let Some(desktop) = delta.desktop {
        if let Some(value) = desktop.telemetry {
            config.desktop.telemetry = Some(value);
        }
        if let Some(value) = desktop.metrics {
            config.desktop.metrics = Some(value);
        }
        if let Some(value) = desktop.tray {
            config.desktop.tray = value;
        }
    }
}
