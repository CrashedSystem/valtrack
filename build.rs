// Compiles res/app.rc into a COFF object and links it, so the .exe carries the
// app icon (Explorer, shortcuts, tray). Only runs for Windows targets; on other
// targets it is a no-op.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".into());
    let prefix = match arch.as_str() {
        "x86" => "i686-w64-mingw32",
        "aarch64" => "aarch64-w64-mingw32",
        _ => "x86_64-w64-mingw32",
    };
    let windres = env::var("WINDRES").unwrap_or_else(|_| format!("{prefix}-windres"));

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let obj = out.join("app.res.o");

    let ok = Command::new(&windres)
        .arg("res/app.rc")
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if ok {
        println!("cargo:rustc-link-arg={}", obj.display());
    } else {
        println!("cargo:warning=windres not found; exe keeps the default icon");
    }
}
