//! The configurator window. Functionally the same as web/index.html, plus
//! built-in automatic layer switching.
use crate::accel::{Curve, accel_at, curve_params};
use crate::actions::*;
use crate::autoswitch::{self, FocusInfo};
use crate::model::*;
use crate::serial::{SerialEvent, SerialLink};
use futures::StreamExt;
use futures::channel::oneshot;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, StyledExt, WindowExt};
use gpui_kit::*;
use gpui_kit::prelude::*;
use gpui_kit::component::Theme;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Layer,
    Btn(usize),
    Wheel(usize),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Layout,
    Tuning,
    Json,
    Switch,
}

#[derive(Clone, Copy, Default)]
struct LiveWheel {
    speed: f64,
    boost: u8,
    raw: u16,
}
#[derive(Default)]
struct Live {
    layer: Option<usize>,
    btn: [bool; NUM_BUTTONS],
    wheel: [LiveWheel; 2],
}

struct Waiter {
    typ: String,
    tx: oneshot::Sender<Result<Value, String>>,
}

struct Recording {
    key: String,
    max_mods: Vec<&'static str>,
}

pub struct JogApp {
    cfg: Config,
    saved_text: String,
    sel_layer: usize,
    target: Target,
    pane: Pane,
    live: Live,
    link: SerialLink,
    connected: bool,
    conn_text: String,
    learn: bool,
    waiters: Vec<Waiter>,
    banner: Option<(String, bool)>,
    busy: bool,
    inputs: HashMap<String, (Entity<InputState>, Subscription)>,
    inputs_for: String,
    json: Entity<EditorState>,
    json_synced: String,
    json_msg: String,
    seq_draft: Option<Vec<String>>,
    recording: Option<Recording>,
    focus: FocusHandle,
    auto_enabled: bool,
    autostart: bool,
    auto_error: Option<String>,
    focused: Option<FocusInfo>,
    recent: Vec<FocusInfo>,
    last_file: Option<PathBuf>,
    _subs: Vec<Subscription>,
}

const BTN_GPIO: [u8; NUM_BUTTONS] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

impl JogApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (link, mut events) = SerialLink::start();
        // Serial events -> UI
        cx.spawn_in(window, async move |this, cx| {
            while let Some(ev) = events.next().await {
                if this.update_in(cx, |this, window, cx| this.on_serial(ev, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Focus watcher -> UI
        let (ftx, mut frx) = futures::channel::mpsc::unbounded::<Result<FocusInfo, String>>();
        std::thread::spawn(move || {
            let t2 = ftx.clone();
            if let Err(e) = autoswitch::watch(move |i| {
                let _ = t2.unbounded_send(Ok(i));
            }) {
                let _ = ftx.unbounded_send(Err(e.to_string()));
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Some(ev) = frx.next().await {
                if this.update_in(cx, |this, _, cx| this.on_focus_change(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Key recording: grab keystrokes before inputs or actions see them
        let weak = cx.entity().downgrade();
        let rec_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            if let Some(app) = weak.upgrade() {
                let handled = app.update(cx, |this, cx| this.on_record_key(&ev.keystroke, window, cx));
                if handled {
                    cx.stop_propagation();
                }
            }
        });

        let json = cx.new(|cx| EditorState::new(window, cx));
        let cfg = Config::defaults();
        let saved_text = cfg.to_text();
        Self {
            cfg,
            saved_text,
            sel_layer: 0,
            target: Target::Layer,
            pane: Pane::Layout,
            live: Live::default(),
            link,
            connected: false,
            conn_text: "Looking for the pad…".into(),
            learn: false,
            waiters: vec![],
            banner: None,
            busy: false,
            inputs: HashMap::new(),
            inputs_for: String::new(),
            json,
            json_synced: String::new(),
            json_msg: String::new(),
            seq_draft: None,
            recording: None,
            focus: cx.focus_handle(),
            auto_enabled: true,
            autostart: autoswitch::autostart_installed(),
            auto_error: None,
            focused: None,
            recent: vec![],
            last_file: None,
            _subs: vec![rec_sub],
        }
    }

    fn text(&self) -> String {
        self.cfg.to_text()
    }
    fn dirty(&self) -> bool {
        self.text() != self.saved_text
    }
    fn layer(&self) -> &Layer {
        &self.cfg.layers[self.sel_layer]
    }
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.cfg.layers[self.sel_layer]
    }
    fn wheel_mut(&mut self, i: usize) -> &mut Wheel {
        let l = self.layer_mut();
        if i == 0 { &mut l.left } else { &mut l.right }
    }
    fn select(&mut self, layer: usize, target: Target, cx: &mut Context<Self>) {
        self.sel_layer = layer.min(self.cfg.layers.len().saturating_sub(1));
        self.target = target;
        self.seq_draft = None;
        self.recording = None;
        cx.notify();
    }

    fn toast(&self, msg: impl Into<SharedString>, bad: bool, window: &mut Window, cx: &mut App) {
        let n = if bad { Notification::error(msg) } else { Notification::success(msg) };
        window.push_notification(n, cx);
    }

    /// Ask before discarding something. Runs `then` if confirmed.
    fn confirm(
        &self,
        msg: &str,
        ok: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let rx = window.prompt(PromptLevel::Warning, msg, None, &[ok, "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if rx.await == Ok(0) {
                let _ = this.update_in(cx, |this, window, cx| then(this, window, cx));
            }
        })
        .detach();
    }

    // ---- Config replacement ------------------------------------------------
    fn set_config(&mut self, text: &str, source: &str, mark_saved: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match Config::parse(text) {
            Ok(c) => {
                self.cfg = c;
                if mark_saved {
                    self.saved_text = self.text();
                }
                if self.sel_layer >= self.cfg.layers.len() {
                    self.sel_layer = 0;
                }
                self.target = Target::Layer;
                self.seq_draft = None;
                self.inputs.clear();
                cx.notify();
                true
            }
            Err(e) => {
                self.toast(format!("Could not read {source}: {e}"), true, window, cx);
                false
            }
        }
    }

    // ---- Serial ------------------------------------------------------------
    fn request(&mut self, cmd: &str, typ: &str, payload: Option<Vec<u8>>, timeout: Duration, cx: &mut Context<Self>) -> Task<Result<Value, String>> {
        let (tx, rx) = oneshot::channel();
        if !self.link.connected() {
            return Task::ready(Err("The pad is not connected".into()));
        }
        self.waiters.push(Waiter { typ: typ.into(), tx });
        self.link.send_line(cmd);
        if let Some(p) = payload {
            self.link.send(p);
        }
        let timer = cx.background_executor().timer(timeout);
        cx.background_executor().spawn(async move {
            match futures::future::select(rx, timer).await {
                futures::future::Either::Left((Ok(r), _)) => r,
                futures::future::Either::Left((Err(_), _)) => Err("Disconnected".into()),
                futures::future::Either::Right(_) => Err("The pad did not answer".into()),
            }
        })
    }

    fn on_serial(&mut self, ev: SerialEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            SerialEvent::Connected(path) => {
                self.connected = true;
                self.conn_text = format!("Connecting… {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let hello = self.request("HELLO", "hello", None, Duration::from_secs(4), cx);
                cx.spawn_in(window, async move |this, cx| {
                    let r = hello.await;
                    let _ = this.update_in(cx, |this, window, cx| this.on_hello(r, window, cx));
                })
                .detach();
            }
            SerialEvent::Disconnected => {
                self.connected = false;
                self.conn_text = if self.link.enabled() { "Not connected - plug in the pad".into() } else { "Disconnected".into() };
                self.live = Live::default();
                for w in self.waiters.drain(..) {
                    let _ = w.tx.send(Err("Disconnected".into()));
                }
            }
            SerialEvent::Log(s) => eprintln!("[pad] {s}"),
            SerialEvent::Message(m) => self.on_message(m, cx),
        }
        cx.notify();
    }

    fn on_hello(&mut self, r: Result<Value, String>, window: &mut Window, cx: &mut Context<Self>) {
        let hello = match r {
            Ok(h) => h,
            Err(e) => {
                if self.connected {
                    self.conn_text = format!("Connected, but the pad didn't answer: {e}");
                }
                cx.notify();
                return;
            }
        };
        let drive = hello["drive"].as_bool().unwrap_or(false);
        self.conn_text = format!("Connected · firmware {}{}", hello["fw"].as_str().unwrap_or("?"), if drive { " · USB drive open" } else { "" });
        self.live.layer = hello["layer"].as_u64().map(|v| v as usize);
        self.link.send_line(if self.learn { "MONITOR 2" } else { "MONITOR 1" });
        let errors: Vec<String> = hello["errors"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect()).unwrap_or_default();
        self.banner = if drive {
            Some(("The pad's USB drive is open. Close it with Fn + button 9 before saving from here.".into(), true))
        } else if !errors.is_empty() {
            Some((format!("The pad reported problems in its layers.json:\n• {}", errors.join("\n• ")), true))
        } else {
            None
        };
        self.send_focus();
        if self.dirty() {
            self.toast("Connected. You have unsaved edits, so the pad's layout was not loaded.", false, window, cx);
        } else {
            self.load_from_pad(window, cx);
        }
        cx.notify();
    }

    fn on_message(&mut self, m: Value, cx: &mut Context<Self>) {
        let typ = m["type"].as_str().unwrap_or("").to_string();
        if let Some(i) = self.waiters.iter().position(|w| w.typ == typ || typ == "error") {
            let w = self.waiters.remove(i);
            if typ == "error" && w.typ != "error" {
                let _ = w.tx.send(Err(m["msg"].as_str().unwrap_or("error").to_string()));
                return;
            }
            let _ = w.tx.send(Ok(m.clone()));
        }
        match typ.as_str() {
            "layer" => {
                let idx = m["index"].as_u64().map(|v| v as usize);
                self.live.layer = idx;
                if let Some(i) = idx {
                    if self.learn && i < self.cfg.layers.len() && i != self.sel_layer {
                        self.select(i, Target::Layer, cx);
                    }
                }
            }
            "btn" => {
                let i = m["i"].as_u64().unwrap_or(99) as usize;
                let down = m["down"].as_bool().unwrap_or(false);
                if i < NUM_BUTTONS {
                    self.live.btn[i] = down;
                    if down && self.learn && self.live.layer == Some(self.sel_layer) && i != 0 {
                        self.select(self.sel_layer, Target::Btn(i), cx);
                    }
                }
            }
            "wheel" => {
                let w = m["w"].as_u64().unwrap_or(9) as usize;
                if w < 2 {
                    let speed = m["speed"].as_f64().or_else(|| m["speed"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0);
                    self.live.wheel[w] = LiveWheel { speed, boost: m["boost"].as_u64().unwrap_or(0) as u8, raw: m["raw"].as_u64().unwrap_or(0) as u16 };
                    if speed > 0.2 && self.learn && self.live.layer == Some(self.sel_layer) && self.target != Target::Wheel(w) {
                        self.select(self.sel_layer, Target::Wheel(w), cx);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn load_from_pad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.request("GET", "config", None, Duration::from_secs(6), cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = t.await;
            let _ = this.update_in(cx, |this, window, cx| match r {
                Ok(v) => {
                    let text = v["text"].as_str().unwrap_or("").to_string();
                    if this.set_config(&text, "the pad", true, window, cx) {
                        if let Some(l) = this.live.layer.filter(|l| *l < this.cfg.layers.len()) {
                            this.sel_layer = l;
                        }
                        let msg = if v["source"] == "default" { "Loaded the built-in defaults from the pad" } else { "Loaded layers.json from the pad" };
                        this.toast(msg, false, window, cx);
                    }
                }
                Err(e) => this.toast(format!("Could not load from the pad: {e}"), true, window, cx),
            });
        })
        .detach();
    }

    fn save_to_pad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let errs = collect_errors(&self.cfg);
        if !errs.is_empty() && !self.busy {
            let n = errs.len();
            return self.confirm(
                &format!("{n} problem(s) found (listed under the pad). The pad skips invalid actions. Save anyway?"),
                "Save anyway",
                window,
                cx,
                |this, window, cx| this.do_save(window, cx),
            );
        }
        self.do_save(window, cx);
    }
    fn do_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text() + "\n";
        let bytes = text.into_bytes();
        self.busy = true;
        let t = self.request(&format!("PUT {}", bytes.len()), "saved", Some(bytes), Duration::from_secs(10), cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let r = t.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match r {
                    Ok(v) => {
                        this.saved_text = this.text();
                        let errs: Vec<String> = v["errors"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect()).unwrap_or_default();
                        if errs.is_empty() {
                            this.banner = None;
                            this.toast("Saved. The pad is using the new layout.", false, window, cx);
                        } else {
                            this.banner = Some((format!("Saved, but the pad reported:\n• {}", errs.join("\n• ")), true));
                            this.toast("Saved with warnings", true, window, cx);
                        }
                    }
                    Err(e) => this.toast(format!("Save failed: {e}"), true, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_connection(&mut self, cx: &mut Context<Self>) {
        if self.link.enabled() {
            self.link.set_enabled(false);
            self.conn_text = "Disconnecting…".into();
        } else {
            self.link.set_enabled(true);
            self.conn_text = "Looking for the pad…".into();
        }
        cx.notify();
    }

    fn set_learn(&mut self, on: bool, cx: &mut Context<Self>) {
        self.learn = on;
        self.link.send_line(if on { "MONITOR 2" } else { "MONITOR 1" });
        cx.notify();
    }

    fn load_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let go = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            let saved = this.saved_text.clone();
            if this.set_config(DEFAULT_LAYERS_JSON, "defaults", false, window, cx) {
                this.saved_text = saved;
                this.toast("Defaults loaded into the editor. Save to pad to apply them.", false, window, cx);
            }
        };
        if self.dirty() {
            self.confirm("Replace your unsaved edits with the built-in defaults?", "Replace", window, cx, go);
        } else {
            go(self, window, cx);
        }
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = std::fs::read_to_string(&path);
            let _ = this.update_in(cx, |this, window, cx| match text {
                Ok(t) => {
                    let saved = this.saved_text.clone();
                    if this.set_config(&t, &path.display().to_string(), false, window, cx) {
                        this.saved_text = saved;
                        this.last_file = Some(path.clone());
                        this.toast(format!("Opened {}. Save to pad to apply it.", path.display()), false, window, cx);
                    }
                }
                Err(e) => this.toast(format!("Could not open {}: {e}", path.display()), true, window, cx),
            });
        })
        .detach();
    }

    fn save_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .last_file
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some("layers.json"));
        let text = self.text() + "\n";
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let r = std::fs::write(&path, text);
            let _ = this.update_in(cx, |this, window, cx| match r {
                Ok(()) => {
                    this.last_file = Some(path.clone());
                    this.toast(format!("Saved {}", path.display()), false, window, cx);
                }
                Err(e) => this.toast(format!("Could not save: {e}"), true, window, cx),
            });
        })
        .detach();
    }

    // ---- Auto-switch -------------------------------------------------------
    fn on_focus_change(&mut self, ev: Result<FocusInfo, String>, cx: &mut Context<Self>) {
        match ev {
            Err(e) => self.auto_error = Some(e),
            Ok(i) => {
                // Focusing this window shouldn't flip the pad away from the app you came from
                if i.pid == Some(std::process::id()) {
                    return;
                }
                self.recent.retain(|r| r.class != i.class);
                self.recent.insert(0, i.clone());
                self.recent.truncate(10);
                self.focused = Some(i);
                self.send_focus();
            }
        }
        cx.notify();
    }
    fn send_focus(&self) {
        if let (true, Some(f)) = (self.auto_enabled, &self.focused) {
            self.link.send_line(&f.app_line());
        }
    }

    // ---- Key recording -----------------------------------------------------
    fn start_recording(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.as_ref().is_some_and(|r| r.key == key) {
            self.recording = None;
        } else {
            self.recording = Some(Recording { key: key.into(), max_mods: vec![] });
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }
    fn on_record_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(rec) = &self.recording else { return false };
        let key = ks.key.as_str();
        if matches!(key, "shift" | "control" | "alt" | "platform" | "super" | "function" | "capslock" | "fn") {
            return true;
        }
        let Some(tok) = key_token(key) else {
            self.toast(format!("Can't record \"{key}\". Type it or use Pick…"), true, window, cx);
            return true;
        };
        let mut parts: Vec<String> = vec![];
        if !MEDIA.contains(&tok.as_str()) {
            let m = &ks.modifiers;
            for (on, name) in [(m.control, "ctrl"), (m.shift, "shift"), (m.alt, "alt"), (m.platform, "gui")] {
                if on {
                    parts.push(name.into());
                }
            }
        }
        parts.push(tok);
        let key = rec.key.clone();
        self.recording = None;
        self.apply_field(&key, &parts.join("+"), window, cx);
        self.inputs.remove(&key); // re-create with the recorded value
        cx.notify();
        true
    }
    fn on_modifiers(&mut self, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rec) = &mut self.recording else { return };
        let now: Vec<&'static str> = [(m.control, "ctrl"), (m.shift, "shift"), (m.alt, "alt"), (m.platform, "gui")]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| n)
            .collect();
        if now.len() > rec.max_mods.len() {
            rec.max_mods = now;
        } else if now.is_empty() && !rec.max_mods.is_empty() {
            // Modifier(s) pressed and released on their own
            let val = MOD_ORDER.iter().filter(|m| rec.max_mods.contains(m)).copied().collect::<Vec<_>>().join("+");
            let key = rec.key.clone();
            self.recording = None;
            self.apply_field(&key, &val, window, cx);
            self.inputs.remove(&key);
        }
        cx.notify();
    }

    // ---- Field edits ---------------------------------------------------------
    fn steps(&self) -> Vec<String> {
        if let Some(d) = &self.seq_draft {
            return d.clone();
        }
        match self.target {
            Target::Btn(i) => split_steps(&self.layer().buttons[i].action),
            _ => vec![],
        }
    }
    fn commit_steps(&mut self, list: Vec<String>, cx: &mut Context<Self>) {
        self.seq_draft = None;
        if let Target::Btn(i) = self.target {
            self.layer_mut().buttons[i].action = join_steps(&list);
        }
        self.inputs.retain(|k, _| !k.starts_with("b.step."));
        cx.notify();
    }

    fn apply_field(&mut self, key: &str, v: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let t = v.trim();
        let opt = |s: &str| if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
        let wi = if let Target::Wheel(w) = self.target { w } else { 0 };
        match key {
            "l.name" => {
                let old = self.layer().name.clone();
                self.layer_mut().name = v.to_string();
                if !old.is_empty() && self.cfg.settings.auto_fallback == old {
                    self.cfg.settings.auto_fallback = v.to_string();
                }
            }
            "l.short" => self.layer_mut().short = v.to_string(),
            "l.apps" => self.layer_mut().apps = v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect(),
            "b.label" => {
                if let Target::Btn(i) = self.target {
                    self.layer_mut().buttons[i].label = v.to_string();
                }
            }
            k if k.starts_with("b.step.") => {
                let idx: usize = k[7..].parse().unwrap_or(0);
                let mut list = self.steps();
                while list.len() <= idx {
                    list.push(String::new());
                }
                list[idx] = t.to_string();
                if let Target::Btn(i) = self.target {
                    self.layer_mut().buttons[i].action = join_steps(&list);
                }
                if self.seq_draft.is_some() {
                    self.seq_draft = Some(list);
                }
            }
            "w.label" => self.wheel_mut(wi).label = v.to_string(),
            "w.steps" => {
                if let Ok(n) = t.parse::<i64>() {
                    self.wheel_mut(wi).steps = Some(n.clamp(1, 512));
                }
            }
            "w.px" => {
                if let Ok(n) = t.parse::<i64>() {
                    self.wheel_mut(wi).px = Some(n);
                }
            }
            "w.ccw" => self.wheel_mut(wi).ccw = opt(v),
            "w.cw" => self.wheel_mut(wi).cw = opt(v),
            "w.fast_ccw" => self.wheel_mut(wi).fast_ccw = opt(v),
            "w.fast_cw" => self.wheel_mut(wi).fast_cw = opt(v),
            "w.accel_start" | "w.accel_full" | "w.accel_curve" | "w.fast_speed" => {
                if let Ok(f) = t.parse::<f64>() {
                    let w = self.wheel_mut(wi);
                    match key {
                        "w.accel_start" => w.accel_start = Some(f),
                        "w.accel_full" => w.accel_full = Some(f),
                        "w.accel_curve" => w.accel_curve = Some(f),
                        _ => w.fast_speed = Some(f),
                    }
                }
            }
            k if k.starts_with("s.") => {
                let s = &mut self.cfg.settings;
                let f = t.parse::<f64>().ok();
                let n = t.parse::<i64>().ok();
                match &k[2..] {
                    "accel_start" => f.map(|f| s.accel_start = f),
                    "accel_full" => f.map(|f| s.accel_full = f),
                    "accel_curve" => f.map(|f| s.accel_curve = f),
                    "fast_speed" => f.map(|f| s.fast_speed = f),
                    "smoothing_ms" => n.map(|n| s.smoothing_ms = n),
                    "hold_release_ms" => n.map(|n| s.hold_release_ms = n),
                    "layer_steps" => n.map(|n| s.layer_steps = n),
                    "brightness" => n.map(|n| s.brightness = n),
                    "dim_after_s" => n.map(|n| s.dim_after_s = n),
                    "sleep_after_s" => n.map(|n| s.sleep_after_s = n),
                    _ => None,
                };
            }
            _ => {}
        }
        cx.notify();
    }

    /// A text input bound to a field key. Created once per selection and kept
    /// in sync with the model whenever it isn't being edited.
    fn input(&mut self, key: &str, value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((e, _)) = self.inputs.get(key) {
            let e = e.clone();
            let focused = e.read(cx).focus_handle(cx).is_focused(window);
            if !focused && e.read(cx).value().as_ref() != value {
                let v = value.to_string();
                e.update(cx, |s, cx| s.set_value(v, window, cx));
            }
            return e;
        }
        let v = value.to_string();
        let ph = placeholder.to_string();
        let e = cx.new(|cx| InputState::new(window, cx).placeholder(ph).default_value(v));
        let k = key.to_string();
        let sub = cx.subscribe_in(&e, window, move |this, input, ev: &InputEvent, window, cx| {
            if let InputEvent::Change = ev {
                let v = input.read(cx).value().to_string();
                this.apply_field(&k, &v, window, cx);
            }
        });
        self.inputs.insert(key.to_string(), (e.clone(), sub));
        e
    }
}

// ===========================================================================
// Rendering
// ===========================================================================
fn hint(s: impl Into<SharedString>, t: &Theme) -> Div {
    div().text_xs().text_color(t.muted_foreground).child(s.into())
}
fn label(s: impl Into<SharedString>, t: &Theme) -> Div {
    div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(t.muted_foreground).mb_1().child(s.into())
}
fn err_text(s: impl Into<SharedString>, t: &Theme) -> Div {
    div().text_xs().text_color(t.danger).child(s.into())
}
fn panel(t: &Theme) -> Div {
    div().bg(t.background).border_1().border_color(t.border).rounded_lg().p_3()
}
fn fmt_num(f: f64) -> String {
    let s = format!("{f:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl JogApp {
    fn field(&mut self, key: &str, title: &str, value: &str, hint_text: &str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let e = self.input(key, value, "", window, cx);
        div().mb_3().child(label(title.to_string(), cx.theme())).child(Input::new(&e).small()).when(!hint_text.is_empty(), |d| d.child(hint(hint_text.to_string(), cx.theme()).mt_1()))
    }

    /// Action text field with Rec + Pick…
    fn action_row(&mut self, key: &str, value: &str, allow_wait: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let e = self.input(key, value, "e.g. ctrl+shift+z", window, cx);
        let err = if value.trim().is_empty() { None } else { action_error(value, allow_wait) };
        let recording = self.recording.as_ref().is_some_and(|r| r.key == key);
        let k1 = key.to_string();
        let k2 = key.to_string();
        let weak = cx.entity().downgrade();
        div()
            .v_flex()
            .gap_1()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(div().flex_1().min_w_0().child(Input::new(&e).small()))
                    .child(
                        Button::new(SharedString::from(format!("rec-{key}")))
                            .small()
                            .label(if recording { "Press keys…" } else { "⏺ Rec" })
                            .when(recording, |b| b.danger())
                            .on_click(cx.listener(move |this, _, window, cx| this.start_recording(&k1, window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("pick-{key}"))).small().label("Pick…").dropdown_menu(move |mut menu, window, cx| {
                            let groups: Vec<(&str, Vec<&'static str>)> = QUICK_PICKS
                                .iter()
                                .map(|(g, l)| (*g, l.to_vec()))
                                .chain(allow_wait.then(|| ("Pause (sequences)", WAIT_PICKS.to_vec())))
                                .collect();
                            for (g, list) in groups {
                                let weak = weak.clone();
                                let k = k2.clone();
                                menu = menu.submenu(g, window, cx, move |mut m, _, _| {
                                    for a in &list {
                                        let (weak, k, a) = (weak.clone(), k.clone(), *a);
                                        m = m.item(PopupMenuItem::new(a).on_click(move |_, window, cx| {
                                            if let Some(app) = weak.upgrade() {
                                                app.update(cx, |this, cx| {
                                                    this.apply_field(&k, if a == "none" { "" } else { a }, window, cx);
                                                    this.inputs.remove(&k);
                                                });
                                            }
                                        }));
                                    }
                                    m
                                });
                            }
                            menu
                        }),
                    ),
            )
            .when_some(err, |d, e| d.child(err_text(e, cx.theme())))
    }

    fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let dot = if self.connected { t.success } else { t.muted_foreground };
        let connected = self.connected;
        let dirty = self.dirty();
        div()
            .h_flex()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(t.border)
            .bg(t.title_bar)
            .child(div().font_weight(FontWeight::BOLD).text_lg().child("🎛️ Jogwheel"))
            .child(div().size_2().rounded_full().bg(dot).ml_2())
            .child(div().text_sm().text_color(t.muted_foreground).child(self.conn_text.clone()))
            .child(
                Button::new("conn")
                    .small()
                    .label(if self.link.enabled() { "Disconnect" } else { "Connect" })
                    .tooltip("While disconnected the pad's port is free for other tools (firmware upload, web configurator)")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_connection(cx))),
            )
            .child(
                Checkbox::new("learn")
                    .label("Pad selects keys")
                    .checked(self.learn)
                    .disabled(!connected)
                    .tooltip("Pressing a pad button selects it here instead of typing (the pad's output is muted)")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_learn(*checked, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("reload").small().label("Reload from pad").disabled(!connected).on_click(cx.listener(|this, _, window, cx| {
                    if this.dirty() {
                        this.confirm("Discard your unsaved edits and reload from the pad?", "Discard", window, cx, |this, window, cx| {
                            this.load_from_pad(window, cx)
                        });
                    } else {
                        this.load_from_pad(window, cx);
                    }
                })),
            )
            .child(
                Button::new("save")
                    .small()
                    .primary()
                    .label(if self.busy { "Saving…" } else if dirty { "Save to pad •" } else { "Save to pad" })
                    .disabled(!connected || self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save_to_pad(window, cx))),
            )
            .child(Button::new("open").small().label("Open file…").on_click(cx.listener(|this, _, window, cx| this.open_file(window, cx))))
            .child(Button::new("savefile").small().label("Save file…").on_click(cx.listener(|this, _, window, cx| this.save_file(window, cx))))
            .child(Button::new("defaults").small().label("Defaults").on_click(cx.listener(|this, _, window, cx| this.load_defaults(window, cx))))
    }

    fn render_layers(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let items: Vec<AnyElement> = self
            .cfg
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let sel = i == self.sel_layer;
                let live = self.live.layer == Some(i);
                div()
                    .id(("layer", i))
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .when(sel, |d| d.bg(t.accent).text_color(t.accent_foreground))
                    .when(!sel, |d| d.hover(|d| d.bg(t.secondary_hover)))
                    .child(div().text_xs().w_4().text_color(t.muted_foreground).child(format!("{}", i + 1)))
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(if l.name.is_empty() { "(unnamed)".to_string() } else { l.name.clone() }))
                    .when(i >= MAX_LAYERS, |d| d.child(div().text_xs().text_color(t.danger).child("ignored")))
                    .when(live, |d| d.child(div().text_xs().text_color(t.success).child("● on pad")))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(i, Target::Layer, cx)))
                    .into_any_element()
            })
            .collect();
        let n = self.cfg.layers.len();
        panel(&t)
            .w(px(220.))
            .flex_shrink_0()
            .v_flex()
            .gap_1()
            .child(div().font_weight(FontWeight::SEMIBOLD).mb_1().child("Layers"))
            .child(div().id("layer-list").v_flex().gap_0p5().children(items))
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_1()
                    .mt_2()
                    .child(Button::new("ly-add").xsmall().label("+ New").on_click(cx.listener(|this, _, _, cx| {
                        let n = this.cfg.layers.len();
                        this.cfg.layers.push(Layer::blank(n + 1));
                        this.select(n, Target::Layer, cx);
                    })))
                    .child(Button::new("ly-dup").xsmall().label("Duplicate").on_click(cx.listener(|this, _, _, cx| {
                        let mut c = this.layer().clone();
                        c.name = format!("{} 2", c.name).chars().take(10).collect();
                        c.apps.clear();
                        let at = this.sel_layer + 1;
                        this.cfg.layers.insert(at, c);
                        this.select(at, Target::Layer, cx);
                    })))
                    .child(Button::new("ly-up").xsmall().label("↑").disabled(self.sel_layer == 0).on_click(cx.listener(|this, _, _, cx| {
                        let i = this.sel_layer;
                        this.cfg.layers.swap(i, i - 1);
                        this.select(i - 1, this.target, cx);
                    })))
                    .child(Button::new("ly-down").xsmall().label("↓").disabled(self.sel_layer + 1 >= n).on_click(cx.listener(|this, _, _, cx| {
                        let i = this.sel_layer;
                        this.cfg.layers.swap(i, i + 1);
                        this.select(i + 1, this.target, cx);
                    })))
                    .child(Button::new("ly-del").xsmall().danger().label("Delete").disabled(n <= 1).on_click(cx.listener(|this, _, window, cx| {
                        let name = this.layer().name.clone();
                        this.confirm(&format!("Delete layer \"{name}\"?"), "Delete", window, cx, |this, _, cx| {
                            let i = this.sel_layer;
                            this.cfg.layers.remove(i);
                            this.select(i.saturating_sub(1), Target::Layer, cx);
                        });
                    }))),
            )
            .child(hint("On the pad: hold the top-left button and turn the left wheel, or press button 2–8 to jump to layers 1–7.", &t).mt_3())
    }

    fn render_knob(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let l = self.layer();
        let w = if i == 0 { &l.left } else { &l.right };
        let lw = self.live.wheel[i];
        let sel = self.target == Target::Wheel(i);
        let moving = lw.speed > 0.05 && self.live.layer == Some(self.sel_layer);
        let label = if w.label.is_empty() { w.summary() } else { w.label.clone() };
        let angle = lw.raw as f32 / 4096.0 * std::f32::consts::TAU;
        let (body, ring, ptr, glow) = (t.secondary, t.blue, t.blue, t.success);
        div()
            .id(("knob", i))
            .v_flex()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.select(this.sel_layer, Target::Wheel(i), cx)))
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let c = b.center();
                        let r = b.size.width.min(b.size.height) / 2.0 - px(6.);
                        if sel || moving {
                            window.paint_quad(fill(Bounds::centered_at(c, size(r * 2.0 + px(10.), r * 2.0 + px(10.))), if sel { ring } else { glow.opacity(0.4) }).corner_radii(r + px(5.)));
                        }
                        window.paint_quad(fill(Bounds::centered_at(c, size(r * 2.0, r * 2.0)), body).corner_radii(r));
                        let mut p = PathBuilder::stroke(px(4.));
                        let (s, co) = angle.sin_cos();
                        p.move_to(point(c.x + r * 0.25 * s, c.y - r * 0.25 * co));
                        p.line_to(point(c.x + r * 0.85 * s, c.y - r * 0.85 * co));
                        if let Ok(path) = p.build() {
                            window.paint_path(path, ptr);
                        }
                    },
                )
                .size(px(96.)),
            )
            .child(div().text_xs().text_color(t.muted_foreground).max_w(px(130.)).truncate().child(label))
    }

    fn render_oled(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let l = self.layer();
        let on_live = self.live.layer == Some(self.sel_layer);
        let (bg, fg) = (hsla(0.58, 0.25, 0.08, 1.0), hsla(0.53, 0.9, 0.75, 1.0));
        let tag = |i: usize| {
            let b = self.live.wheel[i].boost;
            if !on_live { String::new() } else if b == 255 { ">>".into() } else if b > 1 { format!("x{b}") } else { String::new() }
        };
        let sel = self.target == Target::Layer;
        div()
            .id("oled")
            .w(px(200.))
            .h(px(100.))
            .bg(bg)
            .text_color(fg)
            .rounded_md()
            .border_4()
            .border_color(if sel { t.blue } else { hsla(0.6, 0.08, 0.2, 1.0) })
            .p_1()
            .font_family(t.mono_font_family.clone())
            .text_size(px(10.))
            .line_height(px(12.))
            .overflow_hidden()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.select(this.sel_layer, Target::Layer, cx)))
            .child(div().bg(fg).text_color(bg).text_center().truncate().child(format!("{}/{} {}", self.sel_layer + 1, self.cfg.layers.len(), l.name)))
            .child(div().h_flex().justify_between().child(format!("{} {}", l.left.label, tag(0))).child(format!("{} {}", tag(1), l.right.label)))
            .child(div().grid().grid_cols(5).gap_0p5().mt_1().children(l.buttons.iter().take(NUM_BUTTONS).enumerate().map(|(i, b)| {
                let down = on_live && self.live.btn[i];
                div()
                    .h(px(26.))
                    .border_1()
                    .border_color(fg)
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(9.))
                    .overflow_hidden()
                    .when(down, |d| d.bg(fg).text_color(bg))
                    .child(b.label.clone())
            })))
    }

    fn render_pad(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let on_live = self.live.layer == Some(self.sel_layer);
        let keys: Vec<AnyElement> = self
            .layer()
            .buttons
            .iter()
            .take(NUM_BUTTONS)
            .enumerate()
            .map(|(i, b)| {
                let sel = self.target == Target::Btn(i);
                let down = on_live && self.live.btn[i];
                let bad = seq_error(&b.action).is_some();
                div()
                    .id(("key", i))
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .h(px(86.))
                    .min_w_0()
                    .p_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(t.border)
                    .bg(if down { t.success.opacity(0.35) } else { t.secondary })
                    .when(sel, |d| d.border_2().border_color(t.blue))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.secondary_hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(this.sel_layer, Target::Btn(i), cx)))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(if b.label.is_empty() { " ".to_string() } else { b.label.clone() }))
                    .child(
                        div()
                            .text_xs()
                            .font_family(t.mono_font_family.clone())
                            .text_color(if bad { t.danger } else { t.muted_foreground })
                            .max_w_full()
                            .truncate()
                            .child(action_short(&b.action)),
                    )
                    .child(div().text_xs().text_color(t.muted_foreground).opacity(0.7).child(if i == 0 { "Fn".to_string() } else { format!("{}", i + 1) }))
                    .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(format!("GP{}{}", BTN_GPIO[i], if i == 0 { " · also Fn" } else { "" })).build(window, cx))
                    .into_any_element()
            })
            .collect();
        div()
            .v_flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .bg(t.secondary.opacity(0.4))
                    .border_1()
                    .border_color(t.border)
                    .rounded_xl()
                    .p_4()
                    .w(px(580.))
                    .v_flex()
                    .gap_4()
                    .child(div().h_flex().justify_between().items_center().child(self.render_knob(0, cx)).child(self.render_oled(cx)).child(self.render_knob(1, cx)))
                    .child(div().grid().grid_cols(5).gap_2().children(keys)),
            )
            .child(hint("Click a key, a wheel or the screen to edit it. The top-left key is also Fn: tapping it fires its action, holding it switches layers.", &t))
    }

    fn graph(&self, series: Vec<(Option<Wheel>, Curve, Hsla, Option<f64>)>, height: f32, t: &Theme) -> impl IntoElement {
        let max_speed = series.iter().map(|(w, p, _, _)| p.full.max(if w.as_ref().is_some_and(|w| w.has_fast()) { p.fast } else { 0.0 }) * 1.35).fold(3.0f64, f64::max);
        let max_mult = series.iter().map(|(w, _, _, _)| w.as_ref().and_then(|w| w.accel).unwrap_or(1.0)).fold(2.0f64, f64::max);
        let (grid, muted) = (t.border, t.muted_foreground);
        let (pl, pr, pt, pb) = (34.0f32, 10.0f32, 12.0f32, 22.0f32);
        let mult_ticks: Vec<i32> = (1..=max_mult.floor() as i32).collect();
        let speed_ticks: Vec<f64> = (0..).map(|i| i as f64 * 0.5).take_while(|v| *v <= max_speed).collect();
        let s2 = series.clone();
        let ticks2 = mult_ticks.clone();
        div()
            .relative()
            .w_full()
            .h(px(height))
            .bg(t.secondary.opacity(0.4))
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                        let x = |v: f64| b.origin.x + px(pl + (v / max_speed) as f32 * (w - pl - pr));
                        let y = |m: f64| b.origin.y + px(h - pb - ((m - 1.0) / (max_mult - 1.0)) as f32 * (h - pt - pb));
                        for m in &ticks2 {
                            let mut p = PathBuilder::stroke(px(1.));
                            p.move_to(point(b.origin.x + px(pl), y(*m as f64)));
                            p.line_to(point(b.origin.x + px(w - pr), y(*m as f64)));
                            if let Ok(p) = p.build() {
                                window.paint_path(p, grid);
                            }
                        }
                        for (wh, p, color, live) in &s2 {
                            let maxm = wh.as_ref().and_then(|w| w.accel).unwrap_or(1.0);
                            if wh.as_ref().is_some_and(|w| w.has_fast()) {
                                let x0 = x(p.fast);
                                window.paint_quad(fill(Bounds::from_corners(point(x0, b.origin.y + px(pt)), point(b.origin.x + px(w - pr), b.origin.y + px(h - pb))), color.opacity(0.13)));
                            }
                            let mut path = PathBuilder::stroke(px(2.5));
                            for i in 0..=200 {
                                let v = max_speed * i as f64 / 200.0;
                                let pnt = point(x(v), y(accel_at(maxm, p, v)));
                                if i == 0 { path.move_to(pnt) } else { path.line_to(pnt) }
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, *color);
                            }
                            if let Some(s) = live.filter(|s| *s > 0.01) {
                                let v = s.min(max_speed);
                                window.paint_quad(fill(Bounds::centered_at(point(x(v), y(accel_at(maxm, p, v))), size(px(12.), px(12.))), *color).corner_radii(px(6.)));
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(mult_ticks.iter().map(|m| {
                let frac = (*m as f64 - 1.0) / (max_mult - 1.0);
                div()
                    .absolute()
                    .left(px(4.))
                    .bottom(px(pb - 6.0) + (px(height - pt - pb) * frac as f32))
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(format!("x{m}"))
            }))
            .child(
                div()
                    .absolute()
                    .bottom(px(3.))
                    .left(px(pl))
                    .right(px(pr))
                    .h_flex()
                    .justify_between()
                    .text_size(px(10.))
                    .text_color(muted)
                    .children(speed_ticks.iter().step_by(if speed_ticks.len() > 12 { 2 } else { 1 }).map(|v| div().child(format!("{v:.1}")))),
            )
    }

    fn render_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sel_key = format!("{}:{:?}", self.sel_layer, self.target);
        if self.inputs_for != sel_key {
            self.inputs.retain(|k, _| k.starts_with("s."));
            self.inputs_for = sel_key;
        }
        let t = cx.theme().clone();
        let body: AnyElement = match self.target {
            Target::Layer => {
                let l = self.layer().clone();
                div()
                    .child(div().font_weight(FontWeight::SEMIBOLD).mb_3().child(format!("Layer {}", self.sel_layer + 1)))
                    .child(self.field("l.name", "Name (max 10)", &l.name, "", window, cx))
                    .child(self.field("l.short", "Short name (max 4, layer picker grid)", &l.short, "", window, cx))
                    .child(self.field(
                        "l.apps",
                        "Auto-switch apps",
                        &l.apps.join(", "),
                        "Comma separated. Matches the window class (e.g. kdenlive, resolve, firefox). Use title:youtube to match the window title. See the Auto-switch tab for the windows you've used recently.",
                        window,
                        cx,
                    ))
                    .child(hint("Click a key or a wheel on the pad to edit it.", &t))
                    .into_any_element()
            }
            Target::Btn(i) => self.render_button_inspector(i, window, cx).into_any_element(),
            Target::Wheel(w) => self.render_wheel_inspector(w, window, cx).into_any_element(),
        };
        panel(&t).w(px(380.)).flex_shrink_0().h_full().child(div().id("inspector").size_full().overflow_y_scroll().child(body)).into_any_element()
    }

    fn render_button_inspector(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let b = self.layer().buttons[i].clone();
        let name = self.layer().name.clone();
        let mut steps = self.steps();
        if steps.is_empty() {
            steps.push(String::new());
        }
        let n = steps.len();
        let mut rows = div().v_flex().gap_2();
        for (k, st) in steps.iter().enumerate() {
            let row = self.action_row(&format!("b.step.{k}"), st, true, window, cx);
            rows = rows.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_1()
                    .child(div().w_4().pt_1().text_xs().text_color(t.muted_foreground).child(if n > 1 { format!("{}", k + 1) } else { String::new() }))
                    .child(row)
                    .when(n > 1, |d| {
                        d.child(Button::new(("step-up", k)).xsmall().label("↑").disabled(k == 0).on_click(cx.listener(move |this, _, _, cx| {
                            let mut l = this.steps();
                            l.swap(k - 1, k);
                            this.commit_steps(l, cx);
                        })))
                        .child(Button::new(("step-del", k)).xsmall().label("✕").on_click(cx.listener(move |this, _, _, cx| {
                            let mut l = this.steps();
                            l.remove(k);
                            this.commit_steps(l, cx);
                        })))
                    }),
            );
        }
        let full = n >= MAX_SEQ;
        div()
            .child(div().font_weight(FontWeight::SEMIBOLD).mb_3().child(format!("Button {} · {name}", i + 1)))
            .child(self.field("b.label", "Label (shown on the OLED, max 4)", &b.label, "", window, cx))
            .child(label(if n > 1 { format!("Key sequence ({n} steps)") } else { "Action".into() }, &t))
            .child(rows)
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .mt_2()
                    .ml_5()
                    .child(Button::new("add-key").xsmall().label("+ Add key").disabled(full).tooltip("Add a step and record it").on_click(cx.listener(|this, _, window, cx| {
                        let mut l = this.steps();
                        if l.is_empty() {
                            l.push(String::new());
                        }
                        if !l.last().unwrap().is_empty() {
                            l.push(String::new());
                        }
                        let k = l.len() - 1;
                        this.seq_draft = Some(l);
                        this.start_recording(&format!("b.step.{k}"), window, cx);
                    })))
                    .child(Button::new("add-wait").xsmall().label("+ Wait").disabled(full).tooltip("Pause between steps").on_click(cx.listener(|this, _, _, cx| {
                        let mut l: Vec<String> = this.steps().into_iter().filter(|s| !s.is_empty()).collect();
                        let at = l.len().saturating_sub(1);
                        l.insert(at, "wait:50".into());
                        this.commit_steps(l, cx);
                    }))),
            )
            .child(
                hint(
                    if n > 1 {
                        "Steps run in order when you press the button. Each step is tapped, except the last one, which stays held while you hold the button."
                    } else {
                        "Held while the button is held. Modifiers first: ctrl+shift+z. Add a key to make a sequence, e.g. kp_plus then shift+delete."
                    },
                    &t,
                )
                .mt_2(),
            )
            .when(i == 0, |d| d.child(hint("This is also the Fn key: its action fires when you tap and release it.", &t).mt_2()))
            .when(i == 8, |d| d.child(hint("Fn + this button opens the pad's USB drive.", &t).mt_2()))
            .when(i == 9, |d| d.child(hint("Fn + this button held for 3 s reboots into the USB bootloader.", &t).mt_2()))
    }

    fn render_wheel_inspector(&mut self, wi: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let l = self.layer().clone();
        let w = if wi == 0 { l.left.clone() } else { l.right.clone() };
        let mode = w.mode.clone();
        let p = curve_params(Some(&w), &self.cfg.settings);
        let custom = w.has_custom_curve();
        let live = self.live.wheel[wi];
        let on_live = self.live.layer == Some(self.sel_layer);
        let mode_btn = |id: &'static str, m: &'static str, text: &'static str, cx: &mut Context<Self>| {
            Button::new(id).xsmall().label(text).when(mode == m, |b| b.primary()).on_click(cx.listener(move |this, _, _, cx| {
                let w = this.wheel_mut(wi);
                w.mode = m.into();
                if m == "steps" && w.steps.is_none() {
                    w.steps = Some(24);
                }
                if m.starts_with("mouse") && w.px.is_none() {
                    w.px = Some(1000);
                }
                cx.notify();
            }))
        };
        let mut d = div()
            .child(div().font_weight(FontWeight::SEMIBOLD).mb_3().child(format!("{} wheel · {}", if wi == 0 { "Left" } else { "Right" }, l.name)))
            .child(self.field("w.label", "Label (max 8)", &w.label, "", window, cx))
            .child(label("Mode", &t))
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .mb_3()
                    .child(mode_btn("m-steps", "steps", "Steps", cx))
                    .child(mode_btn("m-mx", "mouse_x", "Mouse X", cx))
                    .child(mode_btn("m-my", "mouse_y", "Mouse Y", cx))
                    .child(mode_btn("m-none", "none", "Off", cx)),
            );
        if mode == "steps" {
            let hold_mods: Vec<&str> = w.hold.as_deref().unwrap_or("").split('+').filter_map(|s| mod_canonical(s.trim().to_lowercase().as_str())).collect();
            d = d
                .child(self.field("w.steps", "Steps per turn", &w.steps.unwrap_or(24).to_string(), "Higher = finer. 48 ≈ one step every 7.5°.", window, cx))
                .child(label("↺ Counter-clockwise", &t))
                .child(self.action_row("w.ccw", w.ccw.as_deref().unwrap_or(""), false, window, cx).mb_3())
                .child(label("↻ Clockwise", &t))
                .child(self.action_row("w.cw", w.cw.as_deref().unwrap_or(""), false, window, cx).mb_3())
                .child(label("Hold while turning", &t))
                .child(div().h_flex().gap_3().children(MOD_ORDER.iter().map(|m| {
                    let m = *m;
                    Checkbox::new(SharedString::from(format!("hold-{m}"))).label(m).checked(hold_mods.contains(&m)).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let w = this.wheel_mut(wi);
                        let mut cur: Vec<&str> = w.hold.as_deref().unwrap_or("").split('+').filter_map(|s| mod_canonical(s.trim())).collect();
                        cur.retain(|x| *x != m);
                        if *checked {
                            cur.push(m);
                        }
                        let s = MOD_ORDER.iter().filter(|x| cur.contains(x)).copied().collect::<Vec<_>>().join("+");
                        w.hold = if s.is_empty() { None } else { Some(s) };
                        cx.notify();
                    }))
                })))
                .child(hint("Held down while you turn and let go shortly after you stop, e.g. Alt for an Alt+Tab switcher.", &t).mt_1().mb_3());
        }
        if mode.starts_with("mouse") {
            d = d.child(self.field("w.px", "Pixels per turn", &w.px.unwrap_or(1000).to_string(), "Negative = reversed direction.", window, cx));
        }
        if mode != "none" {
            let accel = w.accel.unwrap_or(1.0);
            let step = if mode == "steps" { 1.0 } else { 0.5 };
            let mut acc = div()
                .border_1()
                .border_color(t.border)
                .rounded_md()
                .p_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).text_sm().mb_2().child("Acceleration"))
                .child(label(format!("Max multiplier: x{}", fmt_num(accel)), &t))
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(Button::new("acc-dn").xsmall().label("−").disabled(accel <= 1.0).on_click(cx.listener(move |this, _, _, cx| {
                            let w = this.wheel_mut(wi);
                            let v = (w.accel.unwrap_or(1.0) - step).max(1.0);
                            w.accel = if v > 1.0 { Some(v) } else { None };
                            cx.notify();
                        })))
                        .child(div().h(px(6.)).flex_1().rounded_full().bg(t.secondary).child(div().h_full().rounded_full().bg(t.blue).w(relative(((accel - 1.0) / 9.0) as f32))))
                        .child(Button::new("acc-up").xsmall().label("+").disabled(accel >= 10.0).on_click(cx.listener(move |this, _, _, cx| {
                            let w = this.wheel_mut(wi);
                            w.accel = Some((w.accel.unwrap_or(1.0) + step).min(10.0));
                            cx.notify();
                        }))),
                )
                .child(hint(if mode == "steps" { "Turning fast repeats each step up to this many times. x1 = off." } else { "Turning fast multiplies mouse speed. x1 = off." }, &t).mt_1().mb_3());
            if mode == "steps" {
                acc = acc
                    .child(label("⏪ Fast counter-clockwise (optional)", &t))
                    .child(self.action_row("w.fast_ccw", w.fast_ccw.as_deref().unwrap_or(""), false, window, cx).mb_1())
                    .child(hint("Replaces ↺ when spinning faster than the fast speed, e.g. shift+left to jump a second.", &t).mb_3())
                    .child(label("⏩ Fast clockwise (optional)", &t))
                    .child(self.action_row("w.fast_cw", w.fast_cw.as_deref().unwrap_or(""), false, window, cx).mb_3());
            }
            acc = acc
                .child(self.graph(vec![(Some(w.clone()), p, t.blue, on_live.then_some(live.speed))], 170.0, &t))
                .child(div().h_flex().justify_between().text_xs().text_color(t.muted_foreground).mt_1().child("turns / second →").child(format!("{:.2} t/s", live.speed)))
                .child(
                    Checkbox::new("custom-curve").label("Custom speed curve for this wheel").checked(custom).mt_3().on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let g = curve_params(None, &this.cfg.settings);
                        let w = this.wheel_mut(wi);
                        if *checked {
                            w.accel_start = Some(g.start);
                            w.accel_full = Some(g.full);
                            w.accel_curve = Some(g.curve);
                            if w.mode == "steps" {
                                w.fast_speed = Some(g.fast);
                            }
                        } else {
                            (w.accel_start, w.accel_full, w.accel_curve, w.fast_speed) = (None, None, None, None);
                        }
                        cx.notify();
                    })),
                );
            if custom {
                acc = acc.child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap_2()
                        .mt_2()
                        .child(self.field("w.accel_start", "Starts at (t/s)", &fmt_num(p.start), "", window, cx))
                        .child(self.field("w.accel_full", "Full at (t/s)", &fmt_num(p.full), "", window, cx))
                        .child(self.field("w.accel_curve", "Curve", &fmt_num(p.curve), "1 linear, 2 gentle", window, cx))
                        .when(mode == "steps", |d| d.child(self.field("w.fast_speed", "Fast at (t/s)", &fmt_num(p.fast), "", window, cx))),
                );
            } else {
                acc = acc.child(hint("Uses the global curve from the Wheel tuning tab.", &t).mt_1());
            }
            d = d.child(acc);
        }
        d
    }

    fn render_tuning(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let s = self.cfg.settings.clone();
        let l = self.layer().clone();
        let on_live = self.live.layer == Some(self.sel_layer);
        let (c1, c2) = (t.blue, t.success);
        let series = vec![
            (Some(l.left.clone()), curve_params(Some(&l.left), &s), c1, on_live.then_some(self.live.wheel[0].speed)),
            (Some(l.right.clone()), curve_params(Some(&l.right), &s), c2, on_live.then_some(self.live.wheel[1].speed)),
        ];
        let fmt_live = |i: usize| {
            if !self.connected {
                return "–".to_string();
            }
            let lw = self.live.wheel[i];
            format!("{:.2} turns/s {}", lw.speed, if lw.boost == 255 { "⏩ fast".into() } else if lw.boost > 1 { format!("· x{}", lw.boost) } else { String::new() })
        };
        let num = |this: &mut Self, key: &str, title: &str, v: String, h: &str, window: &mut Window, cx: &mut Context<Self>| this.field(&format!("s.{key}"), title, &v, h, window, cx);
        let fallback_names: Vec<String> = self.cfg.layers.iter().map(|l| l.name.clone()).collect();
        let fb = s.auto_fallback.clone();
        let weak = cx.entity().downgrade();
        div()
            .v_flex()
            .gap_3()
            .child(hint("These settings apply to every wheel unless a wheel overrides them. Turn a wheel on the pad: the dot shows its live speed on the curve.", &t))
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_2()
                    .child(div().child(label("Left wheel now", &t)).child(div().font_family(t.mono_font_family.clone()).text_sm().child(fmt_live(0))))
                    .child(div().child(label("Right wheel now", &t)).child(div().font_family(t.mono_font_family.clone()).text_sm().child(fmt_live(1)))),
            )
            .child(self.graph(series, 200.0, &t))
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child("wheel speed (turns / second) →")
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(div().text_color(c1).child("■"))
                            .child(format!("left: {} (x{})", if l.left.label.is_empty() { &l.left.mode } else { &l.left.label }, fmt_num(l.left.accel.unwrap_or(1.0))))
                            .child(div().text_color(c2).child("■"))
                            .child(format!("right: {} (x{}) · layer {}", if l.right.label.is_empty() { &l.right.mode } else { &l.right.label }, fmt_num(l.right.accel.unwrap_or(1.0)), l.name)),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_x_3()
                    .child(num(self, "accel_start", "Acceleration starts at (turns/s)", fmt_num(s.accel_start), "Slower than this = 1 action per step, fully precise.", window, cx))
                    .child(num(self, "accel_full", "Full acceleration at (turns/s)", fmt_num(s.accel_full), "From here the wheel's max multiplier applies.", window, cx))
                    .child(num(self, "accel_curve", "Curve shape", fmt_num(s.accel_curve), "1 = linear, 2 = gentle start, 0.5 = kicks in early.", window, cx))
                    .child(num(self, "fast_speed", "Fast jump at (turns/s)", fmt_num(s.fast_speed), "Wheels with fast actions switch to them above this.", window, cx))
                    .child(num(self, "smoothing_ms", "Speed smoothing (ms)", s.smoothing_ms.to_string(), "Higher = steadier speed, slower to react.", window, cx))
                    .child(num(self, "hold_release_ms", "Release held modifiers after (ms)", s.hold_release_ms.to_string(), "For \"hold while turning\" wheels like Alt+Tab.", window, cx)),
            )
            .child(
                div()
                    .h_flex()
                    .gap_4()
                    .child(Checkbox::new("inv-l").label("Invert left wheel").checked(s.invert_left).on_click(cx.listener(|this, c: &bool, _, cx| {
                        this.cfg.settings.invert_left = *c;
                        cx.notify();
                    })))
                    .child(Checkbox::new("inv-r").label("Invert right wheel").checked(s.invert_right).on_click(cx.listener(|this, c: &bool, _, cx| {
                        this.cfg.settings.invert_right = *c;
                        cx.notify();
                    }))),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).mt_2().child("Other"))
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_x_3()
                    .child(num(self, "layer_steps", "Layers per turn (Fn + left wheel)", s.layer_steps.to_string(), "", window, cx))
                    .child(num(self, "brightness", "OLED brightness (1-100)", s.brightness.to_string(), "", window, cx))
                    .child(num(self, "dim_after_s", "Dim screen after (s, 0 = never)", s.dim_after_s.to_string(), "", window, cx))
                    .child(num(self, "sleep_after_s", "Screen off after (s, 0 = never)", s.sleep_after_s.to_string(), "", window, cx)),
            )
            .child(label("When the focused app matches no layer (auto-switch)", &t))
            .child(
                Button::new("fallback")
                    .small()
                    .label(if fb.is_empty() { "Stay on the current layer".to_string() } else { format!("Switch to {fb}") })
                    .dropdown_menu(move |mut menu, _, _| {
                        let items: Vec<(String, String)> =
                            std::iter::once((String::new(), "Stay on the current layer".to_string())).chain(fallback_names.iter().map(|n| (n.clone(), format!("Switch to {n}")))).collect();
                        for (v, text) in items {
                            let weak = weak.clone();
                            menu = menu.item(PopupMenuItem::new(text).on_click(move |_, _, cx| {
                                if let Some(app) = weak.upgrade() {
                                    app.update(cx, |this, cx| {
                                        this.cfg.settings.auto_fallback = v.clone();
                                        cx.notify();
                                    });
                                }
                            }));
                        }
                        menu
                    }),
            )
    }

    fn render_json(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let text = self.text();
        let focused = self.json.read(cx).focus_handle(cx).is_focused(window);
        if !focused && self.json_synced != text {
            let v = text.clone();
            self.json.update(cx, |s, cx| s.set_value(v, window, cx));
            self.json_synced = text;
        }
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .child(hint("This is exactly what gets saved as layers.json. Edit it directly and press Apply. See REFERENCE.txt on the pad's drive for all key names.", &t))
            .child(div().flex_1().min_h(px(400.)).child(Editor::new(&self.json).h_full()))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(Button::new("json-apply").small().primary().label("Apply").on_click(cx.listener(|this, _, window, cx| {
                        let v = this.json.read(cx).value().to_string();
                        let saved = this.saved_text.clone();
                        let (layer, target) = (this.sel_layer, this.target);
                        if this.set_config(&v, "JSON", false, window, cx) {
                            this.saved_text = saved;
                            this.sel_layer = layer.min(this.cfg.layers.len() - 1);
                            let _ = target;
                            this.json_synced.clear();
                            this.json_msg = "Applied - press Save to pad to store it.".into();
                        }
                    })))
                    .child(Button::new("json-revert").small().label("Revert").on_click(cx.listener(|this, _, _, cx| {
                        this.json_synced.clear();
                        this.json_msg.clear();
                        cx.notify();
                    })))
                    .child(hint(self.json_msg.clone(), &t)),
            )
    }

    fn render_switch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let matched = |i: &FocusInfo| -> String {
            match match_app(&self.cfg, &i.class, &i.title) {
                Some(l) => self.cfg.layers[l].name.clone(),
                None if !self.cfg.settings.auto_fallback.is_empty() => format!("{} (fallback)", self.cfg.settings.auto_fallback),
                None => "— (stays)".into(),
            }
        };
        let sel_name = self.layer().name.clone();
        let rows: Vec<AnyElement> = self
            .recent
            .iter()
            .enumerate()
            .map(|(n, i)| {
                let class_word = i.class.split_whitespace().last().unwrap_or("").to_lowercase();
                let already = self.layer().apps.iter().any(|a| a == &class_word);
                let cw = class_word.clone();
                div()
                    .h_flex()
                    .gap_2()
                    .py_1()
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().w(px(180.)).flex_shrink_0().truncate().font_family(t.mono_font_family.clone()).text_xs().child(i.class.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(t.muted_foreground).child(i.title.clone()))
                    .child(div().w(px(120.)).flex_shrink_0().truncate().text_xs().child(matched(i)))
                    .child(
                        Button::new(("add-app", n))
                            .xsmall()
                            .label(if already { "✓ added".to_string() } else { format!("+ {sel_name}") })
                            .disabled(already || class_word.is_empty())
                            .tooltip(format!("Add \"{class_word}\" to the apps of layer {sel_name}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let apps = &mut this.layer_mut().apps;
                                if !apps.contains(&cw) {
                                    apps.push(cw.clone());
                                }
                                this.inputs.remove("l.apps");
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let focused = self.focused.clone();
        div()
            .v_flex()
            .gap_3()
            .child(
                Checkbox::new("auto-on")
                    .label("Switch the pad's layer to match the focused app")
                    .checked(self.auto_enabled)
                    .on_click(cx.listener(|this, c: &bool, _, cx| {
                        this.auto_enabled = *c;
                        this.send_focus();
                        cx.notify();
                    })),
            )
            .child(hint("While this window is open it does the switching itself. Each layer's \"apps\" list decides which apps it is for; the matching happens on the pad, using its saved layers.json.", &t))
            .child(
                Checkbox::new("autostart")
                    .label("Also switch layers when this app is closed (start in the background at login)")
                    .checked(self.autostart)
                    .on_click(cx.listener(|this, c: &bool, window, cx| {
                        match autoswitch::set_autostart(*c) {
                            Ok(()) => {
                                this.autostart = *c;
                                if *c {
                                    let started = std::env::current_exe().ok().and_then(|exe| std::process::Command::new(exe).arg("--autoswitch").spawn().ok()).is_some();
                                    this.toast(if started { "Background switching started, and will start at every login" } else { "Will start at your next login" }, false, window, cx);
                                }
                            }
                            Err(e) => this.toast(format!("Could not change autostart: {e}"), true, window, cx),
                        }
                        cx.notify();
                    })),
            )
            .child(hint("Runs `jogwheel --autoswitch`: no window, it only reports the focused app to the pad and never holds its port.", &t))
            .when_some(self.auto_error.clone(), |d, e| d.child(err_text(e, &t)))
            .child(div().font_weight(FontWeight::SEMIBOLD).mt_2().child("Focused app"))
            .child(match focused {
                Some(i) => div()
                    .v_flex()
                    .gap_1()
                    .child(div().h_flex().gap_2().child(label("Class", &t)).child(div().font_family(t.mono_font_family.clone()).text_sm().child(i.class.clone())))
                    .child(div().h_flex().gap_2().child(label("Title", &t)).child(div().text_sm().truncate().child(i.title.clone())))
                    .child(div().h_flex().gap_2().child(label("Layer", &t)).child(div().text_sm().child(matched(&i)))),
                None => hint("Switch to another app and it will show up here.", &t),
            })
            .child(div().font_weight(FontWeight::SEMIBOLD).mt_2().child("Recent apps"))
            .child(hint(format!("Add an app to the selected layer ({sel_name}) with its button, then Save to pad. Matching is a substring of the class, case-insensitive."), &t))
            .child(div().v_flex().children(rows))
    }
}

impl Render for JogApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let errors = collect_errors(&self.cfg);
        let pane = self.pane;
        let center: AnyElement = match pane {
            Pane::Layout => self.render_pad(cx).into_any_element(),
            Pane::Tuning => self.render_tuning(window, cx).into_any_element(),
            Pane::Json => self.render_json(window, cx).into_any_element(),
            Pane::Switch => self.render_switch(cx).into_any_element(),
        };
        let inspector = self.render_inspector(window, cx);
        div()
            .id("root")
            .track_focus(&self.focus)
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, window, cx| this.on_modifiers(&ev.modifiers, window, cx)))
            .size_full()
            .v_flex()
            .bg(t.secondary.opacity(0.25))
            .text_color(t.foreground)
            .child(self.render_header(cx))
            .when_some(self.banner.clone(), |d, (msg, bad)| {
                d.child(
                    div()
                        .mx_4()
                        .mt_3()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(if bad { t.danger } else { t.primary })
                        .bg(if bad { t.danger.opacity(0.1) } else { t.primary.opacity(0.1) })
                        .text_sm()
                        .h_flex()
                        .child(div().flex_1().child(msg))
                        .child(Button::new("banner-x").xsmall().label("✕").on_click(cx.listener(|this, _, _, cx| {
                            this.banner = None;
                            cx.notify();
                        }))),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .h_flex()
                    .items_start()
                    .gap_3()
                    .p_4()
                    .child(self.render_layers(cx))
                    .child(
                        panel(&t)
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .v_flex()
                            .child(
                                TabBar::new("tabs")
                                    .selected_index(match pane {
                                        Pane::Layout => 0,
                                        Pane::Tuning => 1,
                                        Pane::Json => 2,
                                        Pane::Switch => 3,
                                    })
                                    .children([Tab::new().label("Layout"), Tab::new().label("Wheel tuning"), Tab::new().label("JSON"), Tab::new().label("Auto-switch")])
                                    .on_click(cx.listener(|this, i: &usize, _, cx| {
                                        this.pane = [Pane::Layout, Pane::Tuning, Pane::Json, Pane::Switch][*i];
                                        cx.notify();
                                    })),
                            )
                            .child(div().id("center").flex_1().min_h_0().pt_3().overflow_y_scroll().child(center))
                            .when(!errors.is_empty(), |d| {
                                d.child(div().v_flex().mt_2().pt_2().border_t_1().border_color(t.border).children(errors.iter().map(|e| err_text(format!("• {e}"), &t))))
                            }),
                    )
                    .child(inspector),
            )
    }
}
