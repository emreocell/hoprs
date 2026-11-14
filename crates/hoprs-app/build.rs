//! Windows only: embeds the icon and the version info into `hoprs.exe`. Other targets
//! build exactly as before.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/hoprs.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
}

// `winresource` is a build dependency of Windows hosts only, which is where a Windows
// executable is built; the resource compiler it drives does not exist elsewhere.
#[cfg(windows)]
fn embed_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/hoprs.ico");
    // Version, product name and description come from Cargo.toml (package metadata).
    res.set("ProductName", "hoprs");
    res.set("FileDescription", "hoprs");
    res.set("LegalCopyright", "MIT OR Apache-2.0");
    res.compile().expect("embedding the Windows resources");
}

#[cfg(not(windows))]
fn embed_windows_resources() {
    println!("cargo:warning=Windows icon not embedded: needs a Windows host (winresource)");
}
