fn main() {
    println!("cargo:rerun-if-changed=assets/jogwheel.rc");
    println!("cargo:rerun-if-changed=assets/jogwheel.ico");
    // Windows: embed the app icon (resource 1 is what GPUI loads for its windows)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/jogwheel.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
