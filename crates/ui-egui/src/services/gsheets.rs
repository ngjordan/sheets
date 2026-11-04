//! Host-provided access to cloud spreadsheets: Google Sheets and Excel.
//!
//! The interface is deliberately narrow: everything happens in xlsx, the one
//! format both services and Iridium Sheets speak natively. That keeps the
//! host implementation small (a handful of API calls) and means there is
//! exactly one OOXML implementation in the product.
//!
//! Excel is different in one way that matters: Microsoft Graph can read and
//! write individual cells in a workbook that is open in Excel Online, so a
//! save lands in the live file instead of replacing it on disk. The host
//! does that; the app does not know the difference.
//!
//! The host is expected to run off the UI thread: `fetch` and `push` do
//! network I/O, and the UI calls them on a worker.

use std::fmt;

/// Which cloud service a spreadsheet lives in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provider {
    #[default]
    Google,
    Excel,
}

impl Provider {
    /// The name shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google Sheets",
            Provider::Excel => "Excel",
        }
    }

    /// The name of the service's file list, for a section heading.
    pub fn section(self) -> &'static str {
        match self {
            Provider::Google => "Google Sheets",
            Provider::Excel => "Excel (OneDrive)",
        }
    }
}

/// What the host needs to know about a spreadsheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SheetInfo {
    /// The spreadsheet id (the `d=` part of its URL).
    pub id: String,
    /// The spreadsheet's title.
    pub title: String,
    /// The sheet tabs, in order.
    pub tabs: Vec<String>,
    /// Which service holds it.
    pub provider: Provider,
}

impl SheetInfo {
    pub fn new(id: impl Into<String>, title: impl Into<String>, tabs: Vec<String>) -> Self {
        SheetInfo { id: id.into(), title: title.into(), tabs, provider: Provider::Google }
    }

    pub fn with_provider(mut self, provider: Provider) -> Self {
        self.provider = provider;
        self
    }
}

/// A Google Sheets account the host can reach.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    /// The signed-in account's email address.
    pub email: String,
    /// False when the host has no working credentials, so the UI can offer
    /// "Sign in" rather than failing on the first call.
    pub signed_in: bool,
}

/// The host's Google Sheets backend.
///
/// Every method may fail; the error text is shown to the user as-is, so it
/// should say what went wrong and what to do, not name internal symbols.
pub trait Backend: Send + Sync {
    /// The signed-in account, if any.
    fn account(&self) -> Account;

    /// The spreadsheets the host can offer to open. An empty list with a
    /// successful return means "none", not "failed".
    fn list(&self) -> Result<Vec<SheetInfo>, String>;

    /// Download a spreadsheet as xlsx bytes.
    fn fetch(&self, spreadsheet_id: &str) -> Result<Vec<u8>, String>;

    /// Upload a workbook (as xlsx bytes) into a spreadsheet.
    fn push(&self, spreadsheet_id: &str, xlsx: &[u8], tab: Option<&str>) -> Result<String, String>;

    /// Start the host's sign-in flow. Returns the URL the user must visit;
    /// the host completes the exchange itself afterwards.
    fn sign_in_url(&self) -> Result<String, String>;

    /// Which service this backend talks to.
    fn provider(&self) -> Provider {
        Provider::Google
    }
}

/// The default host: no Google access. The UI reports it as "not connected"
/// rather than pretending the feature is broken.
#[derive(Debug, Default)]
pub struct NoneBackend;

impl Backend for NoneBackend {
    fn account(&self) -> Account {
        Account { email: String::new(), signed_in: false }
    }
    fn list(&self) -> Result<Vec<SheetInfo>, String> {
        Ok(Vec::new())
    }
    fn fetch(&self, _spreadsheet_id: &str) -> Result<Vec<u8>, String> {
        Err("Google Sheets is not connected on this build.".into())
    }
    fn push(&self, _spreadsheet_id: &str, _xlsx: &[u8], _tab: Option<&str>) -> Result<String, String> {
        Err("Google Sheets is not connected on this build.".into())
    }
    fn sign_in_url(&self) -> Result<String, String> {
        Err("Google Sheets is not connected on this build.".into())
    }
}

/// A backend that reports a single connection problem, for hosts that know
/// they are misconfigured (for example, a missing token file).
#[derive(Debug)]
pub struct UnavailableBackend(pub String);

impl fmt::Display for UnavailableBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Backend for UnavailableBackend {
    fn account(&self) -> Account {
        Account { email: String::new(), signed_in: false }
    }
    fn list(&self) -> Result<Vec<SheetInfo>, String> {
        Err(self.0.clone())
    }
    fn fetch(&self, _spreadsheet_id: &str) -> Result<Vec<u8>, String> {
        Err(self.0.clone())
    }
    fn push(&self, _spreadsheet_id: &str, _xlsx: &[u8], _tab: Option<&str>) -> Result<String, String> {
        Err(self.0.clone())
    }
    fn sign_in_url(&self) -> Result<String, String> {
        Err(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_backend_reports_not_connected() {
        let b = NoneBackend;
        assert!(!b.account().signed_in);
        assert!(b.list().unwrap().is_empty());
        assert!(b.fetch("x").unwrap_err().contains("not connected"));
    }

    #[test]
    fn unavailable_backend_repeats_its_message() {
        let b = UnavailableBackend("token expired".into());
        assert!(b.list().unwrap_err().contains("token expired"));
        assert!(b.push("x", b"", None).unwrap_err().contains("token expired"));
    }

    #[test]
    fn sheet_info_keeps_its_tabs_in_order() {
        let s = SheetInfo::new("abc", "Budget", vec!["Sheet1".into(), "Q4".into()]);
        assert_eq!(s.tabs, ["Sheet1", "Q4"]);
        assert_eq!(s.id, "abc");
    }
}
