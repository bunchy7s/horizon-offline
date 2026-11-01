//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
//! The optional Segment Heap manifest is embedded in the same resource compilation.
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let ico = format!("{dir}/../../packaging/icons/horizon.ico");
    println!("cargo:rerun-if-changed={ico}");
    // The game's main thread gets 8 MB of stack on Windows as on macOS and Linux (Windows gives 1
    // MB): splitscreen builds a joining player's `Game`, ~67 KB, deep in the event loop.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let arg = match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => "/STACK:8388608",
            _ => "-Wl,--stack,8388608",
        };
        println!("cargo:rustc-link-arg-bin=horizon-client={arg}");
    }
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if std::env::var_os("CARGO_FEATURE_WINDOWS_SEGMENT_HEAP").is_some() {
            let manifest = format!("{dir}/windows-segment-heap.manifest");
            println!("cargo:rerun-if-changed={manifest}");
            resource.set_manifest_file(&manifest);
        }
        if let Err(e) = resource.compile() {
            panic!("embedding Windows resources: {e}");
        }
    }
}
