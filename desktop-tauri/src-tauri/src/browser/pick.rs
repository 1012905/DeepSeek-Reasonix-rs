// The element picker's shell half: the page script, the answer's allow-list,
// and the re-finding that decides whether an answer is trusted.
use serde_json::{Value, json};

// The page half of the picker, a lone function stringified into the page: it
// draws the highlight and hover card there because a native view sits above
// anything this window's page draws, and reaches no binding of this process.
const PAGE_PICKER: &str = include_str!("elementpick.js");
pub const STOP: &str =
    "(() => { const picker = window.__reasonixElementPicker; if (picker) picker.stop(); })()";
pub const PICK_LIMIT: std::time::Duration = std::time::Duration::from_secs(90);

const TEXT_CLIP: usize = 4000;
const NEARBY_CLIP: usize = 300;
const MARKUP_CLIP: usize = 6000;
const VALUE_CLIP: usize = 500;
const ATTR_LIMIT: usize = 64;
const STYLE_CLIP: usize = 200;
const ELEMENT_STRINGS: &[&str] = &[
    "pageUrl",
    "pageTitle",
    "tagName",
    "role",
    "accessibleName",
    "selector",
    "xpath",
    "text",
    "nearbyText",
    "htmlExcerpt",
];
const STYLE_KEYS: &[&str] = &[
    "backgroundColor",
    "color",
    "display",
    "fontFamily",
    "fontSize",
    "fontWeight",
];
const SELECTOR_BASIS: &[&str] = &["testid", "aria-label", "id", "class"];
const CANCEL_REASONS: &[&str] = &[
    "escape",
    "dismissed",
    "timeout",
    "hidden",
    "navigation",
    "gone",
];

pub fn arm(labels: &Value) -> String {
    format!("({PAGE_PICKER})({labels})")
}

fn clip(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn gone() -> Value {
    json!({ "cancelled": true, "reason": "gone" })
}

/// The page wrote the answer, so the shell decides what an answer is: fields
/// from an allow-list, strings clipped to the page script's own limits, and a
/// shape it cannot read treated as gone.
pub fn clean(answer: &Value) -> Value {
    if !answer.is_object() {
        return gone();
    }
    if answer
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let reason = answer
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| CANCEL_REASONS.contains(r))
            .unwrap_or("gone");
        return json!({ "cancelled": true, "reason": reason });
    }
    let Some(el) = answer.get("element").filter(|e| e.is_object()) else {
        return gone();
    };
    let mut out = serde_json::Map::new();
    for field in ELEMENT_STRINGS {
        if let Some(value) = el.get(field).and_then(Value::as_str) {
            let limit = match *field {
                "htmlExcerpt" => MARKUP_CLIP,
                "nearbyText" => NEARBY_CLIP,
                _ => TEXT_CLIP,
            };
            out.insert((*field).into(), Value::String(clip(value, limit)));
        }
    }
    if let Some(basis) = el.get("selectorBasis").and_then(Value::as_str)
        && SELECTOR_BASIS.contains(&basis)
    {
        out.insert("selectorBasis".into(), Value::String(basis.into()));
    }
    if let Some(attributes) = el.get("attributes").and_then(Value::as_object) {
        let mut kept = serde_json::Map::new();
        for (key, value) in attributes {
            if kept.len() >= ATTR_LIMIT {
                break;
            }
            // A typed value never crosses to the model, even if the page sends one.
            if key == "value" {
                continue;
            }
            if let Some(value) = value.as_str() {
                kept.insert(
                    clip(key, VALUE_CLIP),
                    Value::String(clip(value, VALUE_CLIP)),
                );
            }
        }
        out.insert("attributes".into(), Value::Object(kept));
    }
    if let Some(at) = el.get("capturedAt").and_then(Value::as_f64)
        && at.is_finite()
    {
        out.insert("capturedAt".into(), json!(at));
    }
    if let Some(rect) = el.get("rect").filter(|r| r.is_object()) {
        let numbers: Option<Vec<f64>> = ["x", "y", "width", "height"]
            .iter()
            .map(|key| {
                rect.get(key)
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite())
            })
            .collect();
        if let Some([x, y, width, height]) = numbers.as_deref() {
            out.insert(
                "rect".into(),
                json!({ "x": x, "y": y, "width": width, "height": height }),
            );
        }
    }
    if let Some(style) = el.get("style").and_then(Value::as_object) {
        let mut kept = serde_json::Map::new();
        for key in STYLE_KEYS {
            if let Some(value) = style.get(*key).and_then(Value::as_str) {
                kept.insert((*key).into(), Value::String(clip(value, STYLE_CLIP)));
            }
        }
        out.insert("style".into(), Value::Object(kept));
    }
    if !out.contains_key("selector") || !out.contains_key("tagName") {
        return gone();
    }
    json!({ "cancelled": false, "element": Value::Object(out) })
}

/// Whether a selector still names an element of the tag the pick reported.
pub fn lands_script(selector: &str) -> String {
    let literal = serde_json::to_string(selector).expect("a string serialises");
    format!(
        "(() => {{ const el = document.querySelector({literal}); return el ? el.tagName.toLowerCase() : \"\"; }})()"
    )
}

pub fn lands_match(answer: &str, tag: &str) -> bool {
    let Ok(found) = serde_json::from_str::<String>(answer) else {
        return false;
    };
    found.eq_ignore_ascii_case(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element() -> Value {
        json!({
            "cancelled": false,
            "element": {
                "tagName": "button", "selector": "#go", "text": "Go",
                "pageUrl": "https://example.com/", "role": "button",
                "selectorBasis": "id", "capturedAt": 1700000000000.0,
                "rect": { "x": 1.0, "y": 2.0, "width": 30.0, "height": 10.0 },
                "attributes": { "id": "go", "value": "secret", "class": "btn" },
                "style": { "color": "#FFF", "evil": "x" },
            },
        })
    }

    #[test]
    fn a_good_answer_keeps_only_the_allow_list() {
        let cleaned = clean(&element());
        assert_eq!(cleaned["cancelled"], false);
        let el = &cleaned["element"];
        assert_eq!(el["selector"], "#go");
        assert_eq!(el["attributes"]["id"], "go");
        assert!(el["attributes"].get("value").is_none());
        assert_eq!(el["style"]["color"], "#FFF");
        assert!(el["style"].get("evil").is_none());
        assert_eq!(el["selectorBasis"], "id");
    }

    #[test]
    fn a_shape_without_selector_or_tag_is_gone() {
        let mut answer = element();
        answer["element"]
            .as_object_mut()
            .expect("obj")
            .remove("selector");
        assert_eq!(clean(&answer)["reason"], "gone");
        assert_eq!(clean(&json!(null))["reason"], "gone");
        assert_eq!(clean(&json!({ "cancelled": false }))["reason"], "gone");
    }

    #[test]
    fn cancellation_reasons_stay_within_the_declared_set() {
        assert_eq!(
            clean(&json!({ "cancelled": true, "reason": "escape" }))["reason"],
            "escape"
        );
        assert_eq!(
            clean(&json!({ "cancelled": true, "reason": "made-up" }))["reason"],
            "gone"
        );
        assert_eq!(clean(&json!({ "cancelled": true }))["reason"], "gone");
    }

    #[test]
    fn long_strings_clip_to_the_page_scripts_limits() {
        let mut answer = element();
        answer["element"]["text"] = Value::String("x".repeat(9000));
        answer["element"]["htmlExcerpt"] = Value::String("y".repeat(9000));
        answer["element"]["nearbyText"] = Value::String("z".repeat(900));
        let cleaned = clean(&answer);
        assert_eq!(cleaned["element"]["text"].as_str().expect("s").len(), 4000);
        assert_eq!(
            cleaned["element"]["htmlExcerpt"].as_str().expect("s").len(),
            6000
        );
        assert_eq!(
            cleaned["element"]["nearbyText"].as_str().expect("s").len(),
            300
        );
    }

    #[test]
    fn the_arm_script_calls_the_picker_with_the_labels() {
        let script = arm(&json!({ "color": "颜色" }));
        assert!(script.starts_with("(function pagePicker"));
        assert!(script.ends_with(")({\"color\":\"颜色\"})"));
    }

    #[test]
    fn lands_compares_the_tag_the_page_answers() {
        assert!(lands_match("\"BUTTON\"", "button"));
        assert!(!lands_match("\"DIV\"", "button"));
        assert!(!lands_match("not json", "button"));
        let script = lands_script("#go");
        assert!(script.contains("document.querySelector(\"#go\")"));
    }
}
