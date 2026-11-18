// Capturing the shell window's pixels for the kernel's page. The verb names
// no window: only the asker's own, matched by process and native handle.
const MAX_WIDTH: u32 = 1920;

pub fn scaled(image: image::RgbaImage) -> image::RgbaImage {
    if image.width() <= MAX_WIDTH {
        return image;
    }
    let height = (image.height() as f64 * MAX_WIDTH as f64 / image.width() as f64).round() as u32;
    image::imageops::resize(
        &image,
        MAX_WIDTH,
        height.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

fn png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(bytes)
}

fn candidates() -> Option<Vec<xcap::Window>> {
    let own = std::process::id();
    Some(
        xcap::Window::all()
            .ok()?
            .into_iter()
            .filter(|w| w.pid().ok() == Some(own))
            .collect(),
    )
}

#[cfg(windows)]
fn own_window(hwnd: isize) -> Option<xcap::Window> {
    let mut windows = candidates()?;
    if let Some(at) = windows
        .iter()
        .position(|w| w.id().ok() == Some(hwnd as u32))
    {
        return Some(windows.remove(at));
    }
    windows.into_iter().next()
}

#[cfg(not(windows))]
fn own_window() -> Option<xcap::Window> {
    candidates()?.into_iter().next()
}

fn capture(#[allow(unused_variables)] hwnd: isize) -> Vec<u8> {
    #[cfg(windows)]
    let target = own_window(hwnd);
    #[cfg(not(windows))]
    let target = own_window();
    let Some(target) = target else {
        return Vec::new();
    };
    let Ok(image) = target.capture_image() else {
        return Vec::new();
    };
    if image.width() == 0 || image.height() == 0 {
        return Vec::new();
    }
    png(&scaled(image)).unwrap_or_default()
}

#[tauri::command]
pub async fn window_capture(window: tauri::WebviewWindow) -> Result<tauri::ipc::Response, String> {
    let hwnd = {
        #[cfg(windows)]
        {
            window.hwnd().map(|h| h.0 as isize).unwrap_or(0)
        }
        #[cfg(not(windows))]
        {
            let _ = &window;
            0
        }
    };
    tauri::async_runtime::spawn_blocking(move || tauri::ipc::Response::new(capture(hwnd)))
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_images_scale_to_the_cap_keeping_aspect() {
        let image = image::RgbaImage::new(3840, 2160);
        let scaled = scaled(image);
        assert_eq!(scaled.width(), MAX_WIDTH);
        assert_eq!(scaled.height(), 1080);
    }

    #[test]
    fn small_images_pass_through_untouched() {
        let image = image::RgbaImage::new(800, 600);
        let scaled = scaled(image);
        assert_eq!(scaled.width(), 800);
        assert_eq!(scaled.height(), 600);
    }

    #[test]
    fn a_picture_encodes_as_png() {
        let bytes = png(&image::RgbaImage::new(2, 2)).expect("encodes");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
