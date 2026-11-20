//! Action strings - mirrors parseAction()/parseSequence() in src/keymap.cpp
use crate::model::{Config, MAX_LAYERS, NUM_BUTTONS};

pub const MOD_ORDER: [&str; 4] = ["ctrl", "shift", "alt", "gui"];
pub const MAX_SEQ: usize = 8;

pub fn mod_canonical(s: &str) -> Option<&'static str> {
    Some(match s {
        "ctrl" | "control" => "ctrl",
        "shift" => "shift",
        "alt" | "option" => "alt",
        "gui" | "super" | "win" | "meta" | "cmd" => "gui",
        _ => return None,
    })
}

pub const KEYS: &[&str] = &[
    "space", "enter", "return", "esc", "escape", "tab", "backspace", "delete", "del", "insert", "ins", "home", "end",
    "pageup", "pgup", "pagedown", "pgdn", "up", "down", "left", "right", "printscreen", "prtsc", "scrolllock",
    "pause", "capslock", "numlock", "menu", "minus", "equal", "lbracket", "rbracket", "backslash", "semicolon",
    "quote", "backtick", "comma", "period", "slash", "kp_0", "kp_1", "kp_2", "kp_3", "kp_4", "kp_5", "kp_6", "kp_7",
    "kp_8", "kp_9", "kp_plus", "kp_minus", "kp_multiply", "kp_divide", "kp_enter", "kp_dot",
];
pub const MEDIA: &[&str] = &[
    "vol_up", "vol_down", "mute", "play_pause", "next_track", "prev_track", "stop", "bright_up", "bright_down",
    "calculator", "browser_back", "browser_forward", "browser_refresh", "browser_home", "search",
];
pub const MOUSE: &[&str] = &["mouse_left", "mouse_right", "mouse_middle", "latch_left", "latch_right", "latch_middle"];

/// None if valid, otherwise an error message.
pub fn action_error(s: &str, allow_wait: bool) -> Option<String> {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s == "none" {
        return None;
    }
    if s.contains(',') {
        return Some("key sequences (a, b) only work on buttons - use \"comma\" for the , key".into());
    }
    if let Some(ms) = s.strip_prefix("wait:") {
        if !allow_wait {
            return Some("wait only works in button sequences".into());
        }
        return match ms.parse::<u32>() {
            Ok(n) if (1..=5000).contains(&n) => None,
            _ => Some("wait must be 1-5000 ms".into()),
        };
    }
    let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Some("empty key name (use kp_plus for \"+\")".into());
    }
    let last = parts.pop().unwrap();
    for p in &parts {
        if mod_canonical(p).is_none() {
            return Some(format!("\"{p}\" is not a modifier"));
        }
    }
    let has_mods = !parts.is_empty();
    if mod_canonical(last).is_some() {
        return None;
    }
    let mut ch = last.chars();
    if let (Some(c), None) = (ch.next(), ch.next()) {
        if c > ' ' && c < '\x7f' {
            return None;
        }
    }
    if let Some(n) = last.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) && !last[1..].starts_with('0') {
            return None;
        }
    }
    if KEYS.contains(&last) || MOUSE.contains(&last) {
        return None;
    }
    if MEDIA.contains(&last) {
        return has_mods.then(|| "media keys can't have modifiers".into());
    }
    for dir in ["scroll_up", "scroll_down"] {
        if let Some(rest) = last.strip_prefix(dir) {
            if rest.is_empty() {
                return None;
            }
            return match rest.strip_prefix(':').and_then(|n| n.parse::<u32>().ok()) {
                Some(n) if (1..=127).contains(&n) => None,
                _ => Some("scroll amount must be 1-127".into()),
            };
        }
    }
    Some(format!("unknown key \"{last}\""))
}

pub fn is_mods_only(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    !s.is_empty() && s.split('+').all(|p| mod_canonical(p.trim()).is_some())
}

pub fn split_steps(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        return vec![];
    }
    s.split(',').map(|x| x.trim().to_string()).collect()
}
pub fn join_steps(steps: &[String]) -> String {
    steps.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
}

pub fn seq_error(s: &str) -> Option<String> {
    let steps = split_steps(s);
    if steps.len() > MAX_SEQ {
        return Some(format!("more than {MAX_SEQ} steps"));
    }
    for (k, st) in steps.iter().enumerate() {
        if st.is_empty() {
            return Some("empty step".into());
        }
        if let Some(e) = action_error(st, true) {
            return Some(if steps.len() > 1 { format!("step {}: {e}", k + 1) } else { e });
        }
    }
    None
}

pub fn action_short(s: &str) -> String {
    let st = split_steps(s);
    if st.is_empty() { "—".into() } else { st.join(" › ") }
}

/// All problems in a config, like the web configurator's error list.
pub fn collect_errors(cfg: &Config) -> Vec<String> {
    let mut errs = vec![];
    for (li, l) in cfg.layers.iter().enumerate() {
        let name = if l.name.is_empty() { "?" } else { &l.name };
        let wh = format!("Layer {} ({name})", li + 1);
        for (i, b) in l.buttons.iter().take(NUM_BUTTONS).enumerate() {
            if let Some(e) = seq_error(&b.action) {
                errs.push(format!("{wh} button {}: {e}", i + 1));
            }
        }
        for (side, w) in [("left", &l.left), ("right", &l.right)] {
            if w.mode != "steps" {
                continue;
            }
            for (k, v) in [("ccw", &w.ccw), ("cw", &w.cw), ("fast_ccw", &w.fast_ccw), ("fast_cw", &w.fast_cw), ("hold", &w.hold)] {
                if let Some(e) = v.as_deref().and_then(|v| action_error(v, false)) {
                    errs.push(format!("{wh} {side} wheel {k}: {e}"));
                }
            }
            if let Some(h) = w.hold.as_deref().filter(|h| !h.trim().is_empty()) {
                if action_error(h, false).is_none() && !is_mods_only(h) {
                    errs.push(format!("{wh} {side} wheel hold: modifiers only"));
                }
            }
        }
    }
    let fb = &cfg.settings.auto_fallback;
    if !fb.is_empty() && !cfg.layers.iter().any(|l| l.name.eq_ignore_ascii_case(fb)) {
        errs.push(format!("Settings: auto_fallback \"{fb}\" is not a layer name"));
    }
    if cfg.layers.len() > MAX_LAYERS {
        errs.push("More than 16 layers - the pad only loads the first 16".into());
    }
    errs
}

/// A GPUI key name -> action token (None = can't be sent by the pad)
pub fn key_token(key: &str) -> Option<String> {
    let k = key.to_lowercase();
    let t = match k.as_str() {
        "space" | "enter" | "tab" | "backspace" | "delete" | "insert" | "home" | "end" | "pageup" | "pagedown" | "up"
        | "down" | "left" | "right" | "pause" | "menu" => k.clone(),
        "escape" => "esc".into(),
        "printscreen" | "print" => "printscreen".into(),
        "-" => "minus".into(),
        "=" => "equal".into(),
        "[" => "lbracket".into(),
        "]" => "rbracket".into(),
        "\\" => "backslash".into(),
        ";" => "semicolon".into(),
        "'" => "quote".into(),
        "`" => "backtick".into(),
        "," => "comma".into(),
        "." => "period".into(),
        "/" => "slash".into(),
        "+" => "kp_plus".into(),
        "*" => "kp_multiply".into(),
        _ => {
            let mut c = k.chars();
            match (c.next(), c.next()) {
                (Some(ch), None) if ch.is_ascii_alphanumeric() => k.clone(),
                _ if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => k.clone(),
                _ => return None,
            }
        }
    };
    Some(t)
}

pub const QUICK_PICKS: &[(&str, &[&str])] = &[
    ("Media", MEDIA),
    (
        "Mouse",
        &["mouse_left", "mouse_right", "mouse_middle", "latch_left", "latch_right", "latch_middle", "scroll_up",
          "scroll_down", "ctrl+scroll_up", "ctrl+scroll_down", "alt+scroll_up", "alt+scroll_down", "shift+scroll_up",
          "shift+scroll_down"],
    ),
    ("Navigation", &["left", "right", "up", "down", "shift+left", "shift+right", "alt+left", "alt+right", "home", "end", "pageup", "pagedown"]),
    ("Editing", &["ctrl+c", "ctrl+v", "ctrl+x", "ctrl+z", "ctrl+shift+z", "ctrl+y", "ctrl+s", "ctrl+a", "delete", "backspace", "enter", "esc", "tab", "space"]),
    ("Modifier only", &["ctrl", "shift", "alt", "gui"]),
    ("Nothing", &["none"]),
];
pub const WAIT_PICKS: &[&str] = &["wait:20", "wait:50", "wait:100", "wait:250", "wait:500", "wait:1000"];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_matches_web() {
        for (s, ok) in [
            ("kp_plus, shift+delete", true), ("a,,b", false), ("wait:50, a", true), ("wait:0", false),
            ("a,b,c,d,e,f,g,h,i", false), ("ctrl+c", true), ("", true), ("foo, a", false), ("f13", true),
            ("f25", false), ("vol_up", true), ("ctrl+vol_up", false), ("scroll_down:3", true), ("ctrl", true),
        ] {
            assert_eq!(seq_error(s).is_none(), ok, "{s}: {:?}", seq_error(s));
        }
        assert!(action_error("a, b", false).is_some());
        assert!(action_error("wait:50", false).is_some());
        assert!(Config::defaults().layers.iter().all(|_| true));
        assert!(collect_errors(&Config::defaults()).is_empty());
    }
}
