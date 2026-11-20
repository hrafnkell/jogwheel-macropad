//! Native window tricks GPUI doesn't offer: hiding a window to the tray,
//! showing it again, and telling a real minimize from other "hidden" states.
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

enum Native {
    #[allow(dead_code)]
    X11(u32),
    #[allow(dead_code)]
    Win32(isize),
    Other,
}

fn native(window: &Window) -> Native {
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => Native::X11(h.window as u32),
        Ok(RawWindowHandle::Xcb(h)) => Native::X11(h.window.get()),
        Ok(RawWindowHandle::Win32(h)) => Native::Win32(h.hwnd.get()),
        _ => Native::Other,
    }
}

/// Can this window be hidden and brought back (i.e. tray mode is possible)?
pub fn can_hide(window: &Window) -> bool {
    !matches!(native(window), Native::Other)
}

pub fn hide(window: &Window) {
    match native(window) {
        #[cfg(target_os = "linux")]
        Native::X11(w) => {
            let _ = x11::withdraw(w);
        }
        #[cfg(windows)]
        Native::Win32(h) => unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(h as _, windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE);
        },
        _ => window.minimize_window(),
    }
}

pub fn show(window: &Window) {
    match native(window) {
        #[cfg(target_os = "linux")]
        Native::X11(w) => {
            let _ = x11::map_and_raise(w);
        }
        #[cfg(windows)]
        Native::Win32(h) => unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            ShowWindow(h as _, if IsIconic(h as _) != 0 { SW_RESTORE } else { SW_SHOW });
            SetForegroundWindow(h as _);
        },
        _ => window.activate_window(),
    }
}

/// True only for a real minimize (not another workspace or being covered).
pub fn is_minimized(window: &Window) -> bool {
    match native(window) {
        #[cfg(target_os = "linux")]
        Native::X11(w) => x11::is_hidden(w).unwrap_or(false),
        #[cfg(windows)]
        Native::Win32(h) => unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(h as _) != 0 },
        _ => false,
    }
}

/// Calls `on_minimize` whenever the user minimizes the window. Compositing
/// X11 window managers keep minimized windows mapped, so GPUI's visibility
/// doesn't change; watch the EWMH state instead. Returns false if this isn't
/// an X11 window (GPUI's visibility events cover the other platforms).
pub fn watch_minimize(window: &Window, on_minimize: impl Fn() + Send + 'static) -> bool {
    match native(window) {
        #[cfg(target_os = "linux")]
        Native::X11(w) => {
            std::thread::spawn(move || {
                let _ = x11::watch_state(w, on_minimize);
            });
            true
        }
        _ => {
            let _ = on_minimize;
            false
        }
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;
    use x11rb::rust_connection::RustConnection;

    fn conn() -> Option<&'static (RustConnection, Window)> {
        static C: OnceLock<Option<(RustConnection, Window)>> = OnceLock::new();
        C.get_or_init(|| {
            let (c, screen) = x11rb::connect(None).ok()?;
            let root = c.setup().roots[screen].root;
            Some((c, root))
        })
        .as_ref()
    }
    fn atom(c: &RustConnection, name: &str) -> anyhow::Result<Atom> {
        Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
    }

    /// ICCCM "withdraw": unmap plus a synthetic UnmapNotify to the root, so the
    /// window manager drops it from the taskbar too.
    pub fn withdraw(w: Window) -> anyhow::Result<()> {
        let (c, root) = conn().ok_or_else(|| anyhow::anyhow!("no X11"))?;
        c.unmap_window(w)?;
        let ev = UnmapNotifyEvent { response_type: UNMAP_NOTIFY_EVENT, sequence: 0, event: *root, window: w, from_configure: false };
        c.send_event(false, *root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)?;
        c.flush()?;
        Ok(())
    }

    pub fn map_and_raise(w: Window) -> anyhow::Result<()> {
        let (c, root) = conn().ok_or_else(|| anyhow::anyhow!("no X11"))?;
        c.map_window(w)?;
        let active = atom(c, "_NET_ACTIVE_WINDOW")?;
        let ev = ClientMessageEvent::new(32, w, active, [2u32, 0, 0, 0, 0]); // 2 = from a pager/tray
        c.send_event(false, *root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)?;
        c.flush()?;
        Ok(())
    }

    pub fn watch_state(w: Window, on_minimize: impl Fn()) -> anyhow::Result<()> {
        use x11rb::protocol::Event;
        let (c, _) = x11rb::connect(None)?;
        let state = atom(&c, "_NET_WM_STATE")?;
        let hidden = atom(&c, "_NET_WM_STATE_HIDDEN")?;
        c.change_window_attributes(w, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        c.flush()?;
        let mut was = false;
        loop {
            if let Event::PropertyNotify(p) = c.wait_for_event()? {
                if p.window == w && p.atom == state {
                    let r = c.get_property(false, w, state, AtomEnum::ATOM, 0, 64)?.reply()?;
                    let now = r.value32().is_some_and(|mut v| v.any(|a| a == hidden));
                    if now && !was {
                        on_minimize();
                    }
                    was = now;
                }
            }
        }
    }

    pub fn is_hidden(w: Window) -> anyhow::Result<bool> {
        let (c, _) = conn().ok_or_else(|| anyhow::anyhow!("no X11"))?;
        let state = atom(c, "_NET_WM_STATE")?;
        let hidden = atom(c, "_NET_WM_STATE_HIDDEN")?;
        let r = c.get_property(false, w, state, AtomEnum::ATOM, 0, 64)?.reply()?;
        Ok(r.value32().is_some_and(|mut v| v.any(|a| a == hidden)))
    }
}
