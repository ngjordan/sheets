//! Tests for the Google Sheets connector.
//!
//! The host backend is the part that can be tested without Google: a fake
//! backend stands in for it, and the checks are that the commands reach the
//! right backend calls and that a fetched workbook actually opens.

use super::*;
use crate::services::gsheets::{Account, Backend, Provider, SheetInfo};
use std::sync::{Arc, Mutex};

/// A backend that records what it was asked and answers from a script.
struct FakeBackend {
    sheets: Vec<SheetInfo>,
    xlsx: Mutex<Vec<u8>>,
    calls: Mutex<Vec<String>>,
}

impl FakeBackend {
    fn new() -> Self {
        FakeBackend {
            sheets: vec![SheetInfo::new("abc123", "Budget 2026", vec!["Sheet1".into()])],
            xlsx: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn with_workbook(xlsx: Vec<u8>) -> Self {
        let b = FakeBackend::new();
        *b.xlsx.lock().unwrap() = xlsx;
        b
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Backend for FakeBackend {
    fn account(&self) -> Account {
        Account { email: "test@example.com".into(), signed_in: true }
    }

    fn list(&self) -> Result<Vec<SheetInfo>, String> {
        self.calls.lock().unwrap().push("list".into());
        Ok(self.sheets.clone())
    }

    fn fetch(&self, id: &str) -> Result<Vec<u8>, String> {
        self.calls.lock().unwrap().push(format!("fetch:{id}"));
        Ok(self.xlsx.lock().unwrap().clone())
    }

    fn push(&self, id: &str, xlsx: &[u8], tab: Option<&str>) -> Result<String, String> {
        self.calls.lock().unwrap().push(format!("push:{id}:{}:{}", xlsx.len(), tab.unwrap_or("-")));
        Ok("Budget 2026".into())
    }

    fn sign_in_url(&self) -> Result<String, String> {
        Ok("https://example.com/signin".into())
    }
}

/// A workbook the fake backend returns, built by the engine itself.
fn sample_xlsx() -> Vec<u8> {
    let mut s = iridium_engine::Session::new();
    let r = s.run("file.new", serde_json::json!({})).expect("a new workbook");
    let _ = r;
    let bytes = s.run("file.saveBytes", serde_json::json!({"format": "xlsx"})).expect("encoded");
    let b64 = bytes.get("base64").and_then(serde_json::Value::as_str).expect("base64");
    iridium_engine::io::base64_decode(b64).expect("decoded")
}

fn app_with(backend: Arc<dyn Backend>) -> SheetApp {
    let services = crate::Services { gsheets: vec![backend], ..Default::default() };
    SheetApp::new(iridium_engine::Session::new(), services)
}

#[test]
fn list_asks_the_backend_for_the_spreadsheets() {
    let fake = Arc::new(FakeBackend::new());
    let backend: Arc<dyn Backend> = fake.clone();
    let mut app = app_with(backend);
    super::list(&mut app, Provider::Google);
    // The job runs on a worker; drive it by polling until it finishes.
    for _ in 0..100 {
        super::poll(&mut app);
        if !app.gsheets_sheets.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.gsheets_sheets.len(), 1);
    assert_eq!(app.gsheets_sheets[0].id, "abc123");
    assert_eq!(app.gsheets_sheets[0].title, "Budget 2026");
    assert!(fake.calls().contains(&"list".to_string()));
}

#[test]
fn a_fetched_workbook_opens() {
    let fake = Arc::new(FakeBackend::with_workbook(sample_xlsx()));
    let backend: Arc<dyn Backend> = fake.clone();
    let mut app = app_with(backend);
    super::fetch(&mut app, Provider::Google, "abc123");
    for _ in 0..200 {
        super::poll(&mut app);
        if app.gsheets_current.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.gsheets_current.clone(), Some((Provider::Google, "abc123".to_string())));
    // The workbook is open: the engine has a document.
    assert!(!app.session.documents().is_empty());
    assert!(fake.calls().iter().any(|c| c == "fetch:abc123"));
}

#[test]
fn save_without_a_spreadsheet_is_an_error() {
    let fake = Arc::new(FakeBackend::new());
    let backend: Arc<dyn Backend> = fake;
    let mut app = app_with(backend);
    // No Google Sheet is open, so there is nowhere to save to.
    let r = app.run("ui.gsheets.save", serde_json::json!({}));
    assert!(r.is_err(), "saving without a target must fail");
    assert!(r.unwrap_err().contains("spreadsheet id"));
}
