//! Focused-window watcher for automatic layer switching (X11 and Windows).
//!
//! Reports the focused window every time focus moves, and when the focused
//! window's title changes (so "title:youtube" rules follow browser tab
//! switches). The "class" sent to the pad is the X11 WM_CLASS
//! ("instance Class"); on Windows it is "<program> <window class>", e.g.
//! "kdenlive Qt5QWindowIcon", so the same "apps" rules work on both.
use std::time::Duration;

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

/// Watch focus forever, calling `report` on every change. Returns Err only
/// when focus can't be watched at all (no X11 display, unsupported OS).
pub fn watch(report: impl FnMut(FocusInfo) + Send + 'static) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    return x11::watch(report);
    #[cfg(windows)]
    return win::watch(report);
    #[allow(unreachable_code)]
    {
        let _ = report;
        anyhow::bail!("Automatic layer switching isn't supported on this OS yet.")
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use super::FocusInfo;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask, Window};

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
}

#[cfg(windows)]
mod win {
    use super::FocusInfo;
    use std::cell::RefCell;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    thread_local! {
        static REPORT: RefCell<Option<Box<dyn FnMut(FocusInfo)>>> = RefCell::new(None);
        static LAST: RefCell<FocusInfo> = RefCell::new(FocusInfo::default());
    }

    fn process_name(pid: u32) -> String {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        }
    }

    fn info(hwnd: HWND) -> FocusInfo {
        unsafe {
            let mut title = [0u16; 512];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            let mut class = [0u16; 256];
            let m = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let exe = process_name(pid);
            let class = String::from_utf16_lossy(&class[..m]);
            FocusInfo {
                class: format!("{exe} {class}").trim().to_string(),
                title: String::from_utf16_lossy(&title[..n]),
                pid: Some(pid),
            }
        }
    }

    fn report_foreground() {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return;
        }
        let i = info(hwnd);
        if i.class.is_empty() && i.title.is_empty() {
            return;
        }
        let changed = LAST.with(|l| {
            let mut l = l.borrow_mut();
            if *l == i { false } else { *l = i.clone(); true }
        });
        if changed {
            REPORT.with(|r| {
                if let Some(f) = r.borrow_mut().as_mut() {
                    f(i);
                }
            });
        }
    }

    unsafe extern "system" fn on_event(_hook: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, id_child: i32, _thread: u32, _time: u32) {
        if event == EVENT_OBJECT_NAMECHANGE {
            // Only title changes of the focused top-level window
            if id_object != OBJID_WINDOW || id_child != CHILDID_SELF as i32 || hwnd != unsafe { GetForegroundWindow() } {
                return;
            }
        }
        report_foreground();
    }

    pub fn watch(report: impl FnMut(FocusInfo) + 'static) -> anyhow::Result<()> {
        REPORT.with(|r| *r.borrow_mut() = Some(Box::new(report)));
        unsafe {
            // Out-of-context hooks are delivered to this thread's message loop.
            let a = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, std::ptr::null_mut(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
            let b = SetWinEventHook(EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, std::ptr::null_mut(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
            if a.is_null() || b.is_null() {
                anyhow::bail!("Could not watch window focus (SetWinEventHook failed)");
            }
            report_foreground();
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }
}

/// Autostart entry that runs `jogwheel --autoswitch` at login: an XDG
/// autostart file on Linux, the HKCU Run key on Windows.
#[cfg(not(windows))]
pub fn autostart_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("jogwheel-autoswitch.desktop"))
}
#[cfg(not(windows))]
pub fn autostart_installed() -> bool {
    autostart_path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains("--autoswitch"))
}
#[cfg(not(windows))]
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

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
fn reg(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg").args(args).creation_flags(CREATE_NO_WINDOW).output()
}
#[cfg(windows)]
pub fn autostart_installed() -> bool {
    reg(&["query", RUN_KEY, "/v", "JogwheelAutoswitch"]).is_ok_and(|o| o.status.success())
}
#[cfg(windows)]
pub fn set_autostart(on: bool) -> anyhow::Result<()> {
    let out = if on {
        let exe = std::env::current_exe()?;
        reg(&["add", RUN_KEY, "/v", "JogwheelAutoswitch", "/t", "REG_SZ", "/d", &format!("\"{}\" --autoswitch", exe.display()), "/f"])?
    } else {
        if !autostart_installed() {
            return Ok(());
        }
        reg(&["delete", RUN_KEY, "/v", "JogwheelAutoswitch", "/f"])?
    };
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
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
