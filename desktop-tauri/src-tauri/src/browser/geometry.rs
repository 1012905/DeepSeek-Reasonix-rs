// Where a page's view sits in the rectangle the panel gave it, and how input
// carrying a point is scaled to match. Pure arithmetic, so the placement the
// window draws and the input the kernel sends agree by construction.
use serde::Deserialize;
use serde_json::{Value, json};

/// The agent's page is laid out in a viewport of its own; parked out of sight
/// it keeps that size at a one-pixel corner of the window.
pub const VIEWPORT_WIDTH: f64 = 1280.0;
pub const VIEWPORT_HEIGHT: f64 = 900.0;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn valid_rect(rect: &Rect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|n| n.is_finite())
        && rect.width > 0.0
        && rect.height > 0.0
}

pub fn same_rect(a: Option<Rect>, b: Rect) -> bool {
    a.is_some_and(|a| a == b)
}

pub fn rounded(rect: &Rect) -> Rect {
    Rect {
        x: rect.x.round(),
        y: rect.y.round(),
        width: rect.width.round(),
        height: rect.height.round(),
    }
}

fn sized(value: Option<&Value>) -> bool {
    value.is_some_and(|v| {
        let width = v.get("width").and_then(Value::as_f64);
        let height = v.get("height").and_then(Value::as_f64);
        matches!((width, height), (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0)
    })
}

/// A page laid out in a viewport of its own is shown whole: scaled down to fit
/// and never up, centred, with the view sized to exactly what it draws.
pub fn placement(panel: Rect, emulated: Option<&Value>) -> (Rect, f64) {
    if !sized(emulated) {
        return (panel, 1.0);
    }
    let v = emulated.expect("sized");
    let ew = v["width"].as_f64().expect("sized width");
    let eh = v["height"].as_f64().expect("sized height");
    let scale = (panel.width / ew).min(panel.height / eh).min(1.0);
    let width = (ew * scale).round().max(1.0);
    let height = (eh * scale).round().max(1.0);
    let x = panel.x + ((panel.width - width) / 2.0).round();
    let y = panel.y + ((panel.height - height) / 2.0).round();
    (
        Rect {
            x,
            y,
            width,
            height,
        },
        scale,
    )
}

pub fn is_sized(value: Option<&Value>) -> bool {
    sized(value)
}

// Input that carries a point lands where it is drawn only after the same
// scaling the view was drawn at. The mouse is measured to; a point that lands
// elsewhere silently is worse than a refusal, so the rest are refused scaled.
const POINTED_INPUT: &[&str] = &[
    "Input.dispatchTouchEvent",
    "Input.dispatchDragEvent",
    "Input.emulateTouchFromMouseEvent",
    "Input.synthesizeTapGesture",
    "Input.synthesizeScrollGesture",
    "Input.synthesizePinchGesture",
];

pub fn scale_input(method: &str, params: Value, scale: f64) -> Result<Value, String> {
    if scale == 1.0 {
        return Ok(params);
    }
    if method == "Input.dispatchMouseEvent" {
        let x = params.get("x").and_then(Value::as_f64).unwrap_or(0.0) * scale;
        let y = params.get("y").and_then(Value::as_f64).unwrap_or(0.0) * scale;
        let mut out = params;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("x".into(), json!(x));
            obj.insert("y".into(), json!(y));
        }
        return Ok(out);
    }
    if POINTED_INPUT.contains(&method) {
        return Err(format!(
            "{method} is not available while the page is scaled to fit the panel"
        ));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        }
    }

    #[test]
    fn an_unemulated_page_fills_the_panel_unscaled() {
        let (at, scale) = placement(panel(), None);
        assert_eq!(at, panel());
        assert_eq!(scale, 1.0);
        let zero = json!({ "width": 0, "height": 100 });
        assert_eq!(placement(panel(), Some(&zero)).1, 1.0);
    }

    #[test]
    fn a_wider_page_scales_down_and_centres_never_up() {
        let emulated = json!({ "width": 1600, "height": 900 });
        let (at, scale) = placement(panel(), Some(&emulated));
        assert_eq!(scale, 0.5);
        assert_eq!(at.width, 800.0);
        assert_eq!(at.height, 450.0);
        assert_eq!(at.x, 10.0);
        assert_eq!(at.y, 20.0 + ((600.0f64 - 450.0) / 2.0).round());

        let small = json!({ "width": 400, "height": 300 });
        let (at, scale) = placement(panel(), Some(&small));
        assert_eq!(scale, 1.0);
        assert_eq!(at.width, 400.0);
        assert_eq!(at.x, 10.0 + ((800.0f64 - 400.0) / 2.0).round());
    }

    #[test]
    fn a_mouse_point_is_scaled_to_where_it_is_drawn() {
        let params = json!({ "x": 100, "y": 50, "type": "mousePressed" });
        let scaled = scale_input("Input.dispatchMouseEvent", params, 0.5).expect("scales");
        assert_eq!(scaled["x"], 50.0);
        assert_eq!(scaled["y"], 25.0);
        assert_eq!(scaled["type"], "mousePressed");
    }

    #[test]
    fn pointed_input_other_than_the_mouse_is_refused_while_scaled() {
        let err = scale_input("Input.dispatchTouchEvent", json!({}), 0.5).expect_err("refused");
        assert!(err.contains("Input.dispatchTouchEvent"));
        assert!(scale_input("Input.dispatchKeyEvent", json!({}), 0.5).is_ok());
        assert!(scale_input("Input.dispatchTouchEvent", json!({}), 1.0).is_ok());
    }

    #[test]
    fn a_rect_is_valid_only_when_finite_and_positive() {
        assert!(valid_rect(&panel()));
        assert!(!valid_rect(&Rect {
            width: 0.0,
            ..panel()
        }));
        assert!(!valid_rect(&Rect {
            x: f64::NAN,
            ..panel()
        }));
        assert!(same_rect(Some(panel()), panel()));
        assert!(!same_rect(None, panel()));
    }
}
