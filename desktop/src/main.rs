//! Jogwheel desktop app: configurator + automatic layer switching.
//!
//!   jogwheel                 open the configurator
//!   jogwheel --tray          start hidden in the system tray
//!   jogwheel --autoswitch    no window: just switch layers to match the focused app
//!   jogwheel --install       add an app menu entry and a desktop shortcut
// No console window on Windows (release builds); --help / -v attach to the
// terminal they were started from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod accel;
mod actions;
mod app;
mod autoswitch;
mod install;
mod model;
mod serial;
mod tray;
mod winctl;

use gpui_kit::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "-v" | "--verbose")) {
        unsafe { windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS) };
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "jogwheel {}\n\n  jogwheel                 open the configurator\n  jogwheel --tray          start hidden in the system tray\n  jogwheel --autoswitch    switch layers to match the focused app (no window)\n      -v, --verbose        print every focused window\n  jogwheel --install       add an app menu entry and a desktop shortcut",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--install") {
        if let Err(e) = install::install() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--autoswitch") {
        let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");
        if let Err(e) = autoswitch::run_headless(verbose) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    let start_hidden = args.iter().any(|a| a == "--tray");
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::sync_system_appearance(None, cx);
        let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Jogwheel".into()), ..Default::default() }),
            app_id: Some("jogwheel".into()),
            window_min_size: Some(size(px(1100.), px(640.))),
            icon: Some(std::sync::Arc::new(tray::load_icon(tray::ICON_256))), // X11; Windows uses the exe's icon
            show: !start_hidden,
            focus: !start_hidden,
            ..Default::default()
        };
        gpui_kit::open_window(opts, cx, move |window, cx| cx.new(|cx| app::JogApp::new(start_hidden, window, cx))).expect("failed to open window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
