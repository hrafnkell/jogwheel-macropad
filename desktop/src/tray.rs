//! System tray icon: StatusNotifierItem on Linux (ksni, works with KDE,
//! Cinnamon, XFCE, GNOME + AppIndicator extension), the notification area on
//! Windows (tray-icon). Clicking it shows the window; its menu toggles
//! auto-switching and the pad connection, and quits.
use futures::channel::mpsc::UnboundedSender;

pub const ICON_64: &[u8] = include_bytes!("../assets/icon-64.png");
pub const ICON_256: &[u8] = include_bytes!("../assets/icon-256.png");

pub fn load_icon(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("bundled icon").to_rgba8()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TrayState {
    pub auto: bool,
    pub link_enabled: bool,
    pub connected: bool,
}
impl TrayState {
    fn status(&self) -> &'static str {
        match (self.link_enabled, self.connected) {
            (false, _) => "Pad: disconnected",
            (true, true) => "Pad: connected",
            (true, false) => "Pad: not found",
        }
    }
    fn conn_label(&self) -> &'static str {
        if self.link_enabled { "Disconnect pad" } else { "Connect pad" }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TrayCmd {
    Show,
    /// From the window's minimize watcher, not the tray itself
    Hide,
    ToggleAuto,
    ToggleConnection,
    Quit,
}

pub struct TrayHandle {
    #[cfg(target_os = "linux")]
    handle: ksni::blocking::Handle<linux::KTray>,
    #[cfg(windows)]
    updates: std::sync::mpsc::Sender<TrayState>,
}

impl TrayHandle {
    pub fn update(&self, s: TrayState) {
        #[cfg(target_os = "linux")]
        self.handle.update(|t| t.state = s);
        #[cfg(windows)]
        let _ = self.updates.send(s);
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = s;
    }
}

/// Create the tray icon. None if the desktop has no tray to put it in.
pub fn start(state: TrayState, tx: UnboundedSender<TrayCmd>) -> Option<TrayHandle> {
    #[cfg(target_os = "linux")]
    return linux::start(state, tx);
    #[cfg(windows)]
    return win::start(state, tx);
    #[allow(unreachable_code)]
    {
        let _ = (state, tx);
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use ksni::blocking::TrayMethods;
    use ksni::menu::{CheckmarkItem, StandardItem};

    pub struct KTray {
        pub state: TrayState,
        tx: UnboundedSender<TrayCmd>,
        icons: Vec<ksni::Icon>,
    }
    impl KTray {
        fn send(&self, c: TrayCmd) {
            let _ = self.tx.unbounded_send(c);
        }
    }

    impl ksni::Tray for KTray {
        fn id(&self) -> String {
            "jogwheel".into()
        }
        fn title(&self) -> String {
            "Jogwheel".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "Jogwheel".into(), description: self.state.status().into(), icon_name: String::new(), icon_pixmap: vec![] }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(TrayCmd::Show);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                StandardItem { label: self.state.status().into(), enabled: false, ..Default::default() }.into(),
                ksni::MenuItem::Separator,
                StandardItem { label: "Open Jogwheel".into(), activate: Box::new(|t: &mut Self| t.send(TrayCmd::Show)), ..Default::default() }.into(),
                CheckmarkItem {
                    label: "Switch layers automatically".into(),
                    checked: self.state.auto,
                    activate: Box::new(|t: &mut Self| t.send(TrayCmd::ToggleAuto)),
                    ..Default::default()
                }
                .into(),
                StandardItem { label: self.state.conn_label().into(), activate: Box::new(|t: &mut Self| t.send(TrayCmd::ToggleConnection)), ..Default::default() }.into(),
                ksni::MenuItem::Separator,
                StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.send(TrayCmd::Quit)), ..Default::default() }.into(),
            ]
        }
    }

    fn argb(img: &image::RgbaImage) -> ksni::Icon {
        let mut data = Vec::with_capacity(img.len());
        for p in img.pixels() {
            data.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
        }
        ksni::Icon { width: img.width() as i32, height: img.height() as i32, data }
    }

    pub fn start(state: TrayState, tx: UnboundedSender<TrayCmd>) -> Option<TrayHandle> {
        let big = load_icon(ICON_64);
        let small = image::imageops::resize(&big, 32, 32, image::imageops::FilterType::Lanczos3);
        let tray = KTray { state, tx, icons: vec![argb(&small), argb(&big)] };
        match tray.spawn() {
            Ok(handle) => Some(TrayHandle { handle }),
            Err(e) => {
                eprintln!("tray: {e}");
                None
            }
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, SetTimer, TranslateMessage, WM_TIMER};

    pub fn start(state: TrayState, tx: UnboundedSender<TrayCmd>) -> Option<TrayHandle> {
        let (ok_tx, ok_rx) = mpsc::channel::<bool>();
        let (upd_tx, upd_rx) = mpsc::channel::<TrayState>();
        // The icon and its menu live on their own thread with a Win32 message loop.
        std::thread::spawn(move || {
            let menu = Menu::new();
            let status = MenuItem::new(state.status(), false, None);
            let open = MenuItem::new("Open Jogwheel", true, None);
            let auto = CheckMenuItem::new("Switch layers automatically", true, state.auto, None);
            let conn = MenuItem::new(state.conn_label(), true, None);
            let quit = MenuItem::new("Quit", true, None);
            let sep1 = PredefinedMenuItem::separator();
            let sep2 = PredefinedMenuItem::separator();
            if menu.append_items(&[&status, &sep1, &open, &auto, &conn, &sep2, &quit]).is_err() {
                let _ = ok_tx.send(false);
                return;
            }
            let ids = (open.id().clone(), auto.id().clone(), conn.id().clone(), quit.id().clone());
            let mtx = tx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let cmd = if e.id == ids.0 {
                    TrayCmd::Show
                } else if e.id == ids.1 {
                    TrayCmd::ToggleAuto
                } else if e.id == ids.2 {
                    TrayCmd::ToggleConnection
                } else if e.id == ids.3 {
                    TrayCmd::Quit
                } else {
                    return;
                };
                let _ = mtx.unbounded_send(cmd);
            }));
            let ttx = tx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let show = matches!(e, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. })
                    || matches!(e, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. });
                if show {
                    let _ = ttx.unbounded_send(TrayCmd::Show);
                }
            }));
            let img = load_icon(ICON_64);
            let (w, h) = img.dimensions();
            let Ok(icon) = tray_icon::Icon::from_rgba(img.into_raw(), w, h) else {
                let _ = ok_tx.send(false);
                return;
            };
            let tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Jogwheel").with_icon(icon).with_menu_on_left_click(false).build();
            let Ok(tray) = tray else {
                let _ = ok_tx.send(false);
                return;
            };
            let _ = ok_tx.send(true);
            unsafe {
                // Wake up regularly to apply state changes from the app
                SetTimer(std::ptr::null_mut(), 0, 250, None);
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_TIMER {
                        while let Ok(s) = upd_rx.try_recv() {
                            status.set_text(s.status());
                            auto.set_checked(s.auto);
                            conn.set_text(s.conn_label());
                            let _ = tray.set_tooltip(Some(format!("Jogwheel - {}", s.status())));
                        }
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
        match ok_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(true) => Some(TrayHandle { updates: upd_tx }),
            _ => None,
        }
    }
}
