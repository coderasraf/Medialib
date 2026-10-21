// Embeds the app icon into the Windows executables so Explorer, the taskbar
// and shortcuts show it. No-op on other platforms.
fn main() {
    println!("cargo:rerun-if-changed=assets/medialib.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/medialib.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
