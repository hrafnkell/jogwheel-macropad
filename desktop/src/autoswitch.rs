//! Focused-window watcher (X11) for automatic layer switching.
//!
//! Reports the focused window's WM_CLASS ("instance Class") and title every
//! time focus moves, and when the focused window's title changes (so
//! "title:youtube" rules follow browser tab switches).
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, Window};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FocusInfo {
    pub class: String,
    pub title: String,
    pub pid: Option<u32>,
}

impl FocusInfo {
    pub fn app_line(&self) -> String {
        format!("APP {}\t{}", self.class, self.title).replace('\n', " ")
    }
}

/// Watch focus forever, calling `report` on every change. Reconnects to the
/// X server if it goes away. Returns Err only when there is no X11 display.
pub fn watch(mut report: impl FnMut(FocusInfo)) -> anyhow::Result<()> {
    if std::env::var_os("DISPLAY").is_none() {
        anyhow::bail!("No X11 display found. Automatic layer switching supports X11 sessions only.");
    }
    loop {
        if let Err(e) = watch_once(&mut report) {
            eprintln!("autoswitch: {e}; retrying");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn watch_once(report: &mut impl FnMut(FocusInfo)) -> anyhow::Result<()> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atom = |n: &str| -> anyhow::Result<u32> { Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom) };
    let active_atom = atom("_NET_ACTIVE_WINDOW")?;
    let name_atom = atom("_NET_WM_NAME")?;
    let pid_atom = atom("_NET_WM_PID")?;
    let utf8 = atom("UTF8_STRING")?;

    conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
    conn.flush()?;

    let active = |conn: &x11rb::rust_connection::RustConnection| -> Option<Window> {
        let r = conn.get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
        r.value32()?.next().filter(|w| *w != 0)
    };
    let info = |conn: &x11rb::rust_connection::RustConnection, w: Window| -> FocusInfo {
        let class = conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.value.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect::<Vec<_>>().join(" ")
            })
            .unwrap_or_default();
        let mut title = conn
            .get_property(false, w, name_atom, utf8, 0, 1024)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).into_owned())
            .unwrap_or_default();
        if title.is_empty() {
            title = conn
                .get_property(false, w, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.value).into_owned())
                .unwrap_or_default();
        }
        let pid = conn
            .get_property(false, w, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()));
        FocusInfo { class, title, pid }
    };

    let mut current: Option<Window> = None;
    let mut last = FocusInfo::default();
    let mut refresh = |conn: &x11rb::rust_connection::RustConnection, current: &mut Option<Window>, last: &mut FocusInfo| {
        let w = active(conn);
        if w != *current {
            // Follow title changes of the newly focused window only
            if let Some(old) = *current {
                let _ = conn.change_window_attributes(old, &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT));
            }
            if let Some(new) = w {
                let _ = conn.change_window_attributes(new, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE));
            }
            let _ = conn.flush();
            *current = w;
        }
        if let Some(w) = w {
            let i = info(conn, w);
            if i != *last && !(i.class.is_empty() && i.title.is_empty()) {
                *last = i.clone();
                report(i);
            }
        }
    };
    refresh(&conn, &mut current, &mut last);
    loop {
        let ev = conn.wait_for_event()?;
        if let Event::PropertyNotify(p) = ev {
            let relevant = (p.window == root && p.atom == active_atom)
                || (Some(p.window) == current && (p.atom == name_atom || p.atom == u32::from(AtomEnum::WM_NAME)));
            if relevant {
                refresh(&conn, &mut current, &mut last);
            }
        }
    }
}

/// Autostart entry (~/.config/autostart) that runs `jogwheel --autoswitch`.
pub fn autostart_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("jogwheel-autoswitch.desktop"))
}
pub fn autostart_installed() -> bool {
    autostart_path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains("--autoswitch"))
}
pub fn set_autostart(on: bool) -> anyhow::Result<()> {
    let path = autostart_path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    if on {
        let exe = std::env::current_exe()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "[Desktop Entry]\nType=Application\nName=Jogwheel auto layer switch\n\
                 Comment=Switch macropad layers based on the focused app\nExec=\"{}\" --autoswitch\n\
                 X-GNOME-Autostart-enabled=true\nNoDisplay=false\n",
                exe.display()
            ),
        )?;
    } else if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Headless mode: watch focus and tell the pad, without holding its port.
pub fn run_headless(verbose: bool) -> anyhow::Result<()> {
    use std::sync::{Arc, Mutex};
    let last: Arc<Mutex<Option<FocusInfo>>> = Arc::default();
    // Re-send the current window whenever the pad (re)appears
    let l2 = last.clone();
    std::thread::spawn(move || {
        let mut present = crate::serial::find_port().is_some();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = crate::serial::find_port().is_some();
            if now && !present {
                std::thread::sleep(Duration::from_millis(1500));
                if let Some(i) = l2.lock().unwrap().clone() {
                    crate::serial::send_once(&i.app_line());
                }
            }
            present = now;
        }
    });
    if verbose {
        println!("Watching focus changes (Ctrl+C to stop)...");
    }
    watch(move |i| {
        let ok = crate::serial::send_once(&i.app_line());
        if verbose {
            println!("{} class={:?} title={:?}", if ok { "->" } else { "  " }, i.class, i.title);
        }
        *last.lock().unwrap() = Some(i);
    })
}
