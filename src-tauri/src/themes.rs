//! Colour themes. The palettes live in `src/themes/themes.json`, shared with the frontend (which
//! derives the CSS variables from them). Here the file is read for two things: the list of theme
//! ids that `prefs` accepts for `lightTheme` / `darkTheme`, and, in `cargo test`, the contrast
//! rules of docs/THEMES.md section 6, so CI fails a change that makes a theme hard to read.

use serde::Deserialize;
use std::sync::OnceLock;

const THEMES_JSON: &str = include_str!("../../src/themes/themes.json");

#[derive(Deserialize)]
struct File {
    themes: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    mode: String,
}

fn all() -> &'static [(String, String)] {
    static IDS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    IDS.get_or_init(|| {
        let file: File = serde_json::from_str(THEMES_JSON).expect("src/themes/themes.json is valid");
        file.themes.into_iter().map(|t| (t.id, t.mode)).collect()
    })
}

/// Every theme as `(id, mode)`, mode being "light" or "dark", in the file's order.
pub fn ids() -> Vec<(String, String)> {
    all().to_vec()
}

/// The ids of the themes of one mode ("light" or "dark"), in the file's order.
pub fn ids_for(mode: &str) -> Vec<&'static str> {
    all().iter().filter(|(_, m)| m == mode).map(|(id, _)| id.as_str()).collect()
}

/// The mode of a theme, or `None` for an unknown id.
pub fn mode_of(id: &str) -> Option<&'static str> {
    all().iter().find(|(i, _)| i == id).map(|(_, m)| m.as_str())
}

#[cfg(test)]
mod tests {
    //! docs/THEMES.md section 6, checked on the final colours stored in themes.json.

    use super::THEMES_JSON;
    use serde_json::{Map, Value};
    use std::collections::{HashMap, HashSet};

    const SURFACES: [&str; 4] = ["bg", "chrome", "sidebar", "elevated"];
    const STATUS: [&str; 4] = ["danger", "warning", "success", "info"];
    const SYNTAX: [&str; 9] = ["keyword", "type", "function", "string", "number", "literal", "property", "operator", "comment"];
    const KINDS: [&str; 5] = ["table", "view", "function", "procedure", "trigger"];

    fn themes() -> Vec<Map<String, Value>> {
        let file: Value = serde_json::from_str(THEMES_JSON).expect("themes.json parses");
        file["themes"].as_array().expect("a \"themes\" array").iter().map(|t| t.as_object().expect("each theme is an object").clone()).collect()
    }

    fn is_hex(s: &str) -> bool {
        s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
    }

    /// WCAG 2.x relative luminance of `#rrggbb`.
    fn luminance(hex: &str) -> f64 {
        let channel = |i: usize| {
            let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// One theme's colours by dotted path (`text.hint`, `surface.bg`, `onDanger`, ...).
    struct Theme<'a> {
        id: String,
        obj: &'a Map<String, Value>,
    }

    impl Theme<'_> {
        fn get(&self, path: &str) -> Option<&str> {
            let mut parts = path.split('.');
            let mut v = self.obj.get(parts.next()?)?;
            for p in parts {
                v = v.get(p)?;
            }
            v.as_str().filter(|s| is_hex(s))
        }
    }

    #[test]
    fn theme_list_is_consistent() {
        let all = themes();
        let mut failures = Vec::new();
        let mut modes = HashMap::new();
        for t in &all {
            let id = t.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
            if modes.insert(id.clone(), t.get("mode").and_then(Value::as_str).unwrap_or("").to_string()).is_some() {
                failures.push(format!("{id}: the id is used twice"));
            }
        }
        if all.len() != 19 {
            failures.push(format!("expected 19 themes, found {}", all.len()));
        }
        for required in ["tableboat-dark", "tableboat-light"] {
            if !modes.contains_key(required) {
                failures.push(format!("{required}: missing (it is the default)"));
            }
        }
        for t in &all {
            let id = t.get("id").and_then(Value::as_str).unwrap_or("?");
            let mode = t.get("mode").and_then(Value::as_str).unwrap_or("");
            if !matches!(mode, "light" | "dark") {
                failures.push(format!("{id}: mode must be \"light\" or \"dark\", not {mode:?}"));
            }
            if t.get("name").and_then(Value::as_str).is_none_or(str::is_empty) {
                failures.push(format!("{id}: no name"));
            }
            match t.get("pair").and_then(Value::as_str) {
                None => failures.push(format!("{id}: no pair")),
                Some(pair) => match modes.get(pair) {
                    None => failures.push(format!("{id}: pairs with unknown theme {pair:?}")),
                    Some(m) if m == mode => failures.push(format!("{id} ({mode}): pairs with {pair}, which is also {m}")),
                    Some(_) => {}
                },
            }
            if !t.get("highContrast").is_some_and(Value::is_boolean) {
                failures.push(format!("{id}: highContrast must be true or false"));
            }

            // Every colour is #rrggbb, and every required one is there.
            let theme = Theme { id: id.into(), obj: t };
            let mut required: Vec<String> = Vec::new();
            required.extend(SURFACES.iter().map(|s| format!("surface.{s}")));
            required.extend(["text.primary", "text.secondary", "text.hint", "accent.text", "accent.fill", "accent.on", "dangerFill", "onDanger"].map(String::from));
            required.extend(STATUS.iter().map(|s| format!("status.{s}")));
            required.extend(SYNTAX.iter().map(|s| format!("syntax.{s}")));
            required.extend(KINDS.iter().map(|s| format!("kind.{s}")));
            for path in &required {
                if theme.get(path).is_none() {
                    failures.push(format!("{id}: {path} is missing or not a #rrggbb colour"));
                }
            }
            if t.contains_key("selection") && theme.get("selection").is_none() {
                failures.push(format!("{id}: selection is not a #rrggbb colour"));
            }
            // The close button and error badges always show white text, on a red dark enough for it.
            if theme.get("onDanger") != Some("#ffffff") {
                failures.push(format!("{id}: onDanger must be #ffffff"));
            }
            for group in ["surface", "text", "accent", "status", "syntax", "kind"] {
                for (k, v) in t.get(group).and_then(Value::as_object).into_iter().flatten() {
                    if !v.as_str().is_some_and(is_hex) {
                        failures.push(format!("{id}: {group}.{k} = {v} is not a #rrggbb colour"));
                    }
                }
            }
            for a in t.get("adjusted").and_then(Value::as_array).into_iter().flatten() {
                for k in ["from", "to"] {
                    if !a.get(k).and_then(Value::as_str).is_some_and(is_hex) {
                        failures.push(format!("{id}: adjusted entry {a} has no #rrggbb \"{k}\""));
                    }
                }
            }
            if let Some(o) = t.get("overrides") {
                let ok = o.as_object().is_some_and(|o| o.iter().all(|(k, v)| k.starts_with("--") && v.is_string()));
                if !ok {
                    failures.push(format!("{id}: overrides must map CSS variable names (\"--x\") to strings"));
                }
            }
        }
        assert!(failures.is_empty(), "{} problem(s) in themes.json:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn every_theme_is_readable() {
        let all = themes();
        let mut failures = Vec::new();
        for t in &all {
            let theme = Theme { id: t.get("id").and_then(Value::as_str).unwrap_or("?").into(), obj: t };
            let hc = t.get("highContrast").and_then(Value::as_bool).unwrap_or(false);
            // High Contrast themes: text 15:1, other text-like colours 7:1.
            let (body, ui) = if hc { (15.0, 7.0) } else { (7.0, 4.5) };
            let mut need = |fg: &str, on: &[&str], min: f64| {
                let Some(fg_hex) = theme.get(fg) else { return }; // reported by theme_list_is_consistent
                for bg in on {
                    let Some(bg_hex) = theme.get(bg) else { continue };
                    let r = contrast(fg_hex, bg_hex);
                    if r < min {
                        failures.push(format!("{}: {fg} {fg_hex} on {bg} {bg_hex} is {r:.2}:1, needs {min}:1", theme.id));
                    }
                }
            };
            let every = SURFACES.map(|s| format!("surface.{s}"));
            let every: Vec<&str> = every.iter().map(String::as_str).collect();
            let bg_elevated = ["surface.bg", "surface.elevated"];
            need("text.primary", &every, body);
            need("text.secondary", &every, ui);
            need("text.hint", &every, ui);
            need("accent.text", &bg_elevated, ui);
            need("accent.on", &["accent.fill"], 4.5);
            need("onDanger", &["dangerFill"], 4.5);
            need("text.primary", &["selection"], 4.5);
            for s in STATUS {
                need(&format!("status.{s}"), &bg_elevated, ui);
            }
            for s in SYNTAX {
                need(&format!("syntax.{s}"), &["surface.bg"], if s == "comment" { 3.5 } else { ui });
            }
            for k in KINDS {
                need(&format!("kind.{k}"), &["surface.sidebar", "surface.bg"], 3.0);
            }
        }
        assert!(failures.is_empty(), "{} contrast failure(s):\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn adjusted_lists_only_real_changes() {
        let mut failures = Vec::new();
        for t in &themes() {
            let theme = Theme { id: t.get("id").and_then(Value::as_str).unwrap_or("?").into(), obj: t };
            let mut seen = HashSet::new();
            for a in t.get("adjusted").and_then(Value::as_array).into_iter().flatten() {
                let token = a.get("token").and_then(Value::as_str).unwrap_or("?");
                let (from, to) = (a.get("from").and_then(Value::as_str), a.get("to").and_then(Value::as_str));
                if !seen.insert(token.to_string()) {
                    failures.push(format!("{}: {token} is listed twice in adjusted", theme.id));
                }
                if from == to {
                    failures.push(format!("{}: {token} is listed as adjusted but did not change", theme.id));
                }
                if theme.get(token) != to {
                    failures.push(format!("{}: adjusted says {token} became {to:?}, the theme has {:?}", theme.id, theme.get(token)));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn ids_by_mode() {
        let dark = super::ids_for("dark");
        let light = super::ids_for("light");
        assert_eq!(dark.len() + light.len(), super::ids().len());
        assert!(dark.contains(&"catppuccin-mocha") && light.contains(&"catppuccin-latte"));
        assert_eq!(super::mode_of("tableboat-dark"), Some("dark"));
        assert_eq!(super::mode_of("nope"), None);
    }
}
