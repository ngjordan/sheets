//! Windows only: embed the app icon and version info (VERSIONINFO) into `iridium-sheets.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `IRIDIUM_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).
//!
//! The names come from `branding/brand.env`, the same file the Rust side reads.

use std::{collections::HashMap, env, fs, path::PathBuf};

fn brand() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let path = PathBuf::from(manifest).join("../../branding/brand.env");
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(text) = fs::read_to_string(path) else { return out };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.insert(key.trim().to_string(), value.trim().trim_matches('"').to_string());
        }
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=IRIDIUM_REQUIRE_WINRES");
    let brand = brand();
    let name = brand.get("APP_NAME").cloned().unwrap_or_else(|| "Iridium Sheets".to_string());
    let slug = brand.get("APP_SLUG").cloned().unwrap_or_else(|| "iridium-sheets".to_string());
    let vendor = brand.get("APP_VENDOR").cloned().unwrap_or_else(|| "Iridium Technologies, Inc.".to_string());
    let icon = format!("../../assets/app-icon/{slug}.ico");
    println!("cargo:rerun-if-changed={icon}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(&icon)
        .set("ProductName", &name)
        .set("FileDescription", &format!("{name} spreadsheet"))
        .set("CompanyName", &vendor)
        .set("LegalCopyright", &format!("Copyright (c) the {name} authors. MIT OR Apache-2.0."))
        .set("OriginalFilename", &format!("{slug}.exe"))
        .set("InternalName", &slug);
    if let Err(e) = res.compile() {
        if env::var_os("IRIDIUM_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning={slug}.exe built without icon/version resources: {e}");
    }
}
