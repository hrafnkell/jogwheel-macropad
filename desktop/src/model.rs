//! layers.json data model. Mirrors the format the firmware reads (see
//! config/REFERENCE.txt). Field order here is the order they are written in.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

pub const NUM_BUTTONS: usize = 10;
pub const MAX_LAYERS: usize = 16;
pub const DEFAULT_LAYERS_JSON: &str = include_str!("../../config/default_layers.json");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub invert_left: bool,
    pub invert_right: bool,
    pub accel_start: f64,
    pub accel_full: f64,
    pub accel_curve: f64,
    pub fast_speed: f64,
    pub smoothing_ms: i64,
    pub hold_release_ms: i64,
    pub layer_steps: i64,
    pub brightness: i64,
    pub dim_after_s: i64,
    pub sleep_after_s: i64,
    pub auto_fallback: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            invert_left: false,
            invert_right: false,
            accel_start: 0.4,
            accel_full: 1.5,
            accel_curve: 1.0,
            fast_speed: 1.8,
            smoothing_ms: 50,
            hold_release_ms: 700,
            layer_steps: 10,
            brightness: 100,
            dim_after_s: 30,
            sleep_after_s: 300,
            auto_fallback: "General".into(),
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Layer {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub left: Wheel,
    #[serde(default)]
    pub right: Wheel,
    #[serde(default)]
    pub buttons: Vec<Button>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Wheel {
    #[serde(default)]
    pub label: String,
    #[serde(default = "mode_none")]
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ccw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accel: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_ccw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_cw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accel_start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accel_full: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accel_curve: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_speed: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
fn mode_none() -> String {
    "none".into()
}
impl Default for Wheel {
    fn default() -> Self {
        Self {
            label: String::new(),
            mode: mode_none(),
            steps: None,
            px: None,
            ccw: None,
            cw: None,
            hold: None,
            accel: None,
            fast_ccw: None,
            fast_cw: None,
            accel_start: None,
            accel_full: None,
            accel_curve: None,
            fast_speed: None,
            extra: Map::new(),
        }
    }
}
impl Wheel {
    pub fn has_fast(&self) -> bool {
        self.mode == "steps" && (nonempty(&self.fast_cw) || nonempty(&self.fast_ccw))
    }
    pub fn has_custom_curve(&self) -> bool {
        self.accel_start.is_some() || self.accel_full.is_some() || self.accel_curve.is_some() || self.fast_speed.is_some()
    }
    pub fn summary(&self) -> String {
        match self.mode.as_str() {
            "steps" => format!("{} / {}", opt_or_dash(&self.ccw), opt_or_dash(&self.cw)),
            "mouse_x" => format!("mouse X {}px", self.px.unwrap_or(1000)),
            "mouse_y" => format!("mouse Y {}px", self.px.unwrap_or(1000)),
            _ => "off".into(),
        }
    }
}
fn nonempty(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.trim().is_empty())
}
fn opt_or_dash(s: &Option<String>) -> String {
    match s.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => "—".into(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Button {
    #[serde(default)]
    pub label: String,
    /// Key sequence: steps separated by commas. Arrays are accepted on load.
    #[serde(default, deserialize_with = "string_or_array")]
    pub action: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
fn string_or_array<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Array(a) => a.iter().filter_map(|v| v.as_str()).map(str::trim).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

impl Layer {
    pub fn blank(n: usize) -> Self {
        Self {
            name: format!("Layer {n}"),
            short: format!("L{n}"),
            buttons: vec![Button::default(); NUM_BUTTONS],
            ..Default::default()
        }
    }
}

impl Config {
    /// Parse layers.json text (comments allowed, like the firmware).
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c: Config = serde_json::from_str(&strip_comments(text)).map_err(|e| e.to_string())?;
        if c.layers.is_empty() {
            return Err("no layers found".into());
        }
        for l in &mut c.layers {
            while l.buttons.len() < NUM_BUTTONS {
                l.buttons.push(Button::default());
            }
        }
        Ok(c)
    }
    pub fn defaults() -> Config {
        Config::parse(DEFAULT_LAYERS_JSON).expect("built-in defaults are valid")
    }
    pub fn to_text(&self) -> String {
        pretty(&serde_json::to_value(self).expect("serializable"))
    }
}

/// JSON with // and /* */ comments -> plain JSON
pub fn strip_comments(t: &str) -> String {
    let b: Vec<char> = t.chars().collect();
    let (mut out, mut i, mut in_str) = (String::with_capacity(t.len()), 0, false);
    while i < b.len() {
        let c = b[i];
        let n = b.get(i + 1).copied();
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = n {
                    out.push(n);
                }
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == '/' && n == Some('/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && n == Some('*') {
            i += 2;
            while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Pretty JSON that keeps small objects (buttons, wheels) and short arrays on
/// one line - the same style as the web configurator.
pub fn pretty(v: &Value) -> String {
    let mut s = String::new();
    write_value(v, 0, &mut s);
    s
}
fn is_flat(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.values().all(|x| !x.is_object() && !x.is_array()),
        Value::Array(a) => a.iter().all(|x| !x.is_object() && !x.is_array()),
        _ => true,
    }
}
/// Numbers as the web configurator writes them: 4 rather than 4.0
fn num(x: &Value) -> String {
    match x.as_f64() {
        Some(f) if x.is_f64() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        _ => x.to_string(),
    }
}
fn one_line(v: &Value) -> String {
    match v {
        Value::Object(m) if m.is_empty() => "{}".into(),
        Value::Object(m) => format!(
            "{{ {} }}",
            m.iter().map(|(k, x)| format!("{}: {}", Value::String(k.clone()), num(x))).collect::<Vec<_>>().join(", ")
        ),
        Value::Array(a) => format!("[{}]", a.iter().map(num).collect::<Vec<_>>().join(", ")),
        x => x.to_string(),
    }
}
fn write_value(v: &Value, ind: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match v {
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            if is_flat(v) {
                let one = one_line(v);
                if one.len() <= 120 {
                    out.push_str(&one);
                    return;
                }
            }
            out.push_str("{\n");
            for (i, (k, x)) in m.iter().enumerate() {
                out.push_str(&pad(ind + 1));
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_value(x, ind + 1, out);
                if i + 1 < m.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(ind));
            out.push('}');
        }
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            if is_flat(v) {
                let one = one_line(v);
                if one.len() < 100 {
                    out.push_str(&one);
                    return;
                }
            }
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                out.push_str(&pad(ind + 1));
                write_value(x, ind + 1, out);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(ind));
            out.push(']');
        }
        x => out.push_str(&num(x)),
    }
}

/// Same rule as matchApp() in the firmware: substring of the window class, or
/// "title:" prefix for a substring of the title. Returns the layer index.
pub fn match_app(cfg: &Config, class: &str, title: &str) -> Option<usize> {
    let (cls, ttl) = (class.to_lowercase(), title.to_lowercase());
    cfg.layers.iter().take(MAX_LAYERS).position(|l| {
        l.apps.iter().take(8).any(|p| {
            let p = p.to_lowercase();
            match p.strip_prefix("title:") {
                Some(t) => !t.is_empty() && ttl.contains(t),
                None => !p.is_empty() && cls.contains(&p),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_roundtrip() {
        let c = Config::defaults();
        assert_eq!(c.layers.len(), 7);
        let again = Config::parse(&c.to_text()).unwrap();
        assert_eq!(c, again);
    }
    #[test]
    fn comments_and_arrays() {
        let c = Config::parse(r#"{ // hi
            "layers": [ { "name": "A", /* x */ "buttons": [ { "label": "x", "action": ["a", "b"] } ] } ] }"#)
        .unwrap();
        assert_eq!(c.layers[0].buttons[0].action, "a, b");
        assert_eq!(c.layers[0].buttons.len(), 10);
    }
}
