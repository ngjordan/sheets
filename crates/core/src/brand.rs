//! The app's branding, in one place.
//!
//! `branding/brand.env` at the repository root is the source of truth. This
//! crate's build script reads it at compile time, so changing the name, the
//! vendor or the app id is a one-file edit plus a rebuild.
//!
//! The file is missing when this crate is packaged on its own (crates.io), so
//! the build script falls back to the defaults below.

/// The product name the user sees: window title, menus, the About box.
pub const NAME: &str = env!("IRIDIUM_BRAND_APP_NAME");
/// The short name used in file names: `iridium-sheets`, `iridium-sheets.exe`.
pub const SLUG: &str = env!("IRIDIUM_BRAND_APP_SLUG");
/// The installed identity: bundle id, desktop entry name, hicolor icon names.
pub const ID: &str = env!("IRIDIUM_BRAND_APP_ID");
/// The copyright holder.
pub const VENDOR: &str = env!("IRIDIUM_BRAND_APP_VENDOR");
/// The product home page.
pub const HOMEPAGE: &str = env!("IRIDIUM_BRAND_APP_HOMEPAGE");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brand_is_read_from_the_brand_file() {
        assert!(!NAME.is_empty(), "the app needs a name");
        assert!(!SLUG.is_empty(), "the app needs a file-name slug");
        assert!(!VENDOR.is_empty(), "the app needs a vendor");
        assert!(HOMEPAGE.starts_with("http"), "the home page must be a URL");
        // The id is a reverse-DNS name: the macOS bundle id and the Linux
        // desktop entry name are built from it.
        assert!(ID.contains('.'), "the app id must be reverse DNS");
        assert!(ID.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'), "the app id must be plain");
    }
}
