// Where the two things this shell launches live. A dev build reads them from
// the working tree; a packaged one from its resources, where the host binary
// and the built page sit outside any archive a child could not be spawned from.
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

const HOST: &str = "reasonix-studio-host";

pub struct Layout {
    pub host: PathBuf,
    pub page: PathBuf,
    pub logs: PathBuf,
    pub config: PathBuf,
}

fn host_name() -> String {
    if cfg!(windows) {
        format!("{HOST}.exe")
    } else {
        HOST.to_string()
    }
}

pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Layout> {
    let dev = tauri::is_dev();
    let tree = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let resources = if dev {
        tree.clone()
    } else {
        app.path().resource_dir()?
    };
    let host = std::env::var_os("REASONIX_STUDIO_HOST")
        .map(PathBuf::from)
        .unwrap_or_else(|| resources.join("bin").join(host_name()));
    let page = std::env::var_os("REASONIX_STUDIO_PAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if dev {
                tree.join("..").join("dist")
            } else {
                resources.join("page")
            }
        });
    Ok(Layout {
        host,
        page,
        logs: app.path().app_log_dir()?,
        config: app.path().app_config_dir()?,
    })
}
