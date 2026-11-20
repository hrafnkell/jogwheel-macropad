//! Jogwheel desktop app: configurator + automatic layer switching.
//!
//!   jogwheel                 open the configurator
//!   jogwheel --autoswitch    no window: just switch layers to match the focused app
mod accel;
mod actions;
mod app;
mod autoswitch;
mod model;
mod serial;

use gpui_kit::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "jogwheel {}\n\n  jogwheel                 open the configurator\n  jogwheel --autoswitch    switch layers to match the focused app (no window)\n      -v, --verbose        print every focused window",
            env!("CARGO_PKG_VERSION")
        );
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

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::sync_system_appearance(None, cx);
        let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Jogwheel".into()), ..Default::default() }),
            app_id: Some("jogwheel".into()),
            window_min_size: Some(size(px(1100.), px(640.))),
            ..Default::default()
        };
        gpui_kit::open_window(opts, cx, |window, cx| cx.new(|cx| app::JogApp::new(window, cx))).expect("failed to open window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
