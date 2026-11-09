//! Reads `branding/brand.env` and hands its values to the crate as env vars.
//!
//! The file lives at the repository root so the packaging scripts can read the
//! same source with `set -a; . branding/brand.env; set +a`.

use std::{env, fs, path::PathBuf};

/// Used when the file is missing, which happens when this crate is packaged on
/// its own. Keeps `cargo package` and `cargo publish` working.
const DEFAULTS: &[(&str, &str)] = &[
    ("APP_NAME", "Sheets"),
    ("APP_SLUG", "iridium-sheets"),
    ("APP_ID", "ph.sheets"),
    ("APP_VENDOR", "Iridium Technologies, Inc."),
    ("APP_HOMEPAGE", "https://github.com/ngjordan/iridium-sheets"),
];

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let brand = PathBuf::from(manifest).join("../../branding/brand.env");
    println!("cargo:rerun-if-changed={}", brand.display());

    let mut values: Vec<(String, String)> = DEFAULTS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if let Ok(text) = fs::read_to_string(&brand) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').to_string();
            match values.iter().position(|(k, _)| *k == key) {
                Some(i) => values[i].1 = value,
                None => values.push((key, value)),
            }
        }
    }
    for (key, value) in &values {
        println!("cargo:rustc-env=IRIDIUM_BRAND_{key}={value}");
    }
}
