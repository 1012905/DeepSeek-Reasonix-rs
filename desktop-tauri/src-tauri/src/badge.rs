// The unread mark on the application's icon, in each platform's own form: a
// dock label on macOS, a taskbar overlay on Windows, a count elsewhere.
use tauri::image::Image;
use tauri::{Runtime, WebviewWindow};

const SIZE: u32 = 16;
const DOT_RADIUS: f64 = 4.5;
const DOT_RING: f64 = 5.75;

/// A red dot inside a thin white ring, edges smoothed by coverage, so it reads
/// on any taskbar colour and over any icon. Straight (not premultiplied) RGBA.
pub fn overlay_pixels() -> Vec<u8> {
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    let centre = SIZE as f64 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f64 + 0.5 - centre).powi(2) + (y as f64 + 0.5 - centre).powi(2)).sqrt();
            let outer = (DOT_RING + 0.5 - d).clamp(0.0, 1.0);
            if outer == 0.0 {
                continue;
            }
            let inner = (DOT_RADIUS + 0.5 - d).clamp(0.0, 1.0);
            let mix = |channel: f64| (255.0 + (channel - 255.0) * inner).round() as u8;
            let at = ((y * SIZE + x) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&[
                mix(224.0),
                mix(64.0),
                mix(64.0),
                (255.0 * outer).round() as u8,
            ]);
        }
    }
    pixels
}

pub fn set<R: Runtime>(window: &WebviewWindow<R>, unread: bool) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    return window.set_badge_label(unread.then(|| "•".to_string()));
    #[cfg(windows)]
    return window.set_overlay_icon(unread.then(|| Image::new_owned(overlay_pixels(), SIZE, SIZE)));
    #[cfg(not(any(windows, target_os = "macos")))]
    return window.set_badge_count(unread.then_some(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_is_red_at_its_centre_and_clear_at_its_corners() {
        let pixels = overlay_pixels();
        let at = |x: u32, y: u32| {
            &pixels[((y * SIZE + x) * 4) as usize..((y * SIZE + x) * 4 + 4) as usize]
        };
        assert_eq!(at(8, 8), &[224, 64, 64, 255]);
        assert_eq!(at(0, 0)[3], 0);
        let ring = at(8, 2);
        assert!(
            ring[3] > 0 && ring[1] > 64,
            "the ring is lighter than the dot: {ring:?}"
        );
    }
}
