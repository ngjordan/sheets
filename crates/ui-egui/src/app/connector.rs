//! Connector commands: pull rows from an external source into the sheet.
//!
//! The host does the network I/O (services::connector::Backend), so the work
//! runs on a worker thread and the result is collected on the next frame.
//! The rows land in a new sheet as a table, the way a data import does.

use super::super::services::connector::{Backend, Connector, Rows, Source};
use super::super::{Json, Notice, NoticeKind, SheetApp, clean_error, json};
use std::sync::{Arc, Mutex};

/// What a finished host call produced.
#[derive(Debug)]
pub enum Outcome {
    /// The connectors the host offers.
    List(Vec<Connector>),
    /// The rows a connector returned, ready to be written.
    Fetched { connector: Connector, rows: Rows },
    /// A connector was saved or removed.
    Saved(String),
    /// The call failed; the text is shown to the user.
    Failed(String),
}

/// Work the host thread does, so the UI thread never waits on the network.
type Job = Box<dyn FnOnce(&dyn Backend) -> Outcome + Send + 'static>;

/// A pending job and its result.
#[derive(Default)]
pub struct Pending {
    job: Option<Job>,
    result: Option<Outcome>,
}

/// The pending connector work, if any.
///
/// An `Arc<Mutex<..>>` rather than a bare `Mutex`: the worker thread needs
/// its own handle, and a bare mutex would keep a borrow of the app alive
/// across the mutable use that follows.
pub type State = Arc<Mutex<Pending>>;

/// Run the pending job on a worker and collect its result.
pub fn poll(app: &mut SheetApp) {
    let state = app.connector_state.clone();
    let job = {
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        s.job.take()
    };
    if let Some(job) = job {
        let backend = app.services.connectors.first().cloned();
        let state_for_worker = state.clone();
        std::thread::spawn(move || {
            let out = match backend {
                Some(b) => job(b.as_ref()),
                None => Outcome::Failed("connectors are not available in this build".into()),
            };
            let mut s = state_for_worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            s.result = Some(out);
        });
    }
    // Collect a finished result.
    let outcome = {
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        s.result.take()
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Outcome::List(connectors) => {
            app.connectors = connectors;
        }
        Outcome::Fetched { connector, rows } => {
            let note = if rows.note.is_empty() { format!("{}", rows.len()) } else { rows.note.clone() };
            match write_rows(app, &connector, &rows) {
                Ok(sheet) => {
                    app.notice =
                        Some(Notice { text: format!("Pulled {note} from {} into {sheet}.", connector.name), kind: NoticeKind::Info, action: None });
                }
                Err(e) => app.message = Some((iridium_core::brand::NAME.into(), clean_error(&e))),
            }
        }
        Outcome::Saved(name) => {
            app.notice = Some(Notice { text: format!("Saved the connector {name}."), kind: NoticeKind::Info, action: None });
        }
        Outcome::Failed(message) => {
            app.message = Some((iridium_core::brand::NAME.into(), clean_error(&message)));
        }
    }
}

/// Write the rows into a new sheet, as a table.
///
/// The sheet is named after the connector, and a second pull does not pile up
/// sheets with the same name.
fn write_rows(app: &mut SheetApp, connector: &Connector, rows: &Rows) -> Result<String, String> {
    if rows.header.is_empty() {
        return Err(format!("{} returned no columns.", connector.name));
    }
    let width = rows.header.len().max(rows.rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut values: Vec<Vec<Json>> = Vec::with_capacity(rows.rows.len() + 1);
    values.push(rows.header.iter().map(|h| json!(h)).collect());
    for row in &rows.rows {
        let mut cells: Vec<Json> = row.iter().map(|v| json!(v)).collect();
        cells.resize(width, Json::Null);
        values.push(cells);
    }
    let name = sheet_name(&connector.name);
    let inserted = app.run("home.insertSheet", json!({"name": name}))?;
    let sheet = inserted.get("name").and_then(Json::as_str).unwrap_or(&name).to_string();
    // The values go in as literal text: a cell that holds "1E5" or "=" stays
    // exactly what the server sent, and a server cannot write a formula that
    // reads the workbook.
    app.run("range.setValues", json!({"range": "A1", "values": values, "literal": true}))?;
    let last = column_letters(width);
    let end = format!("A1:{last}{}", values.len());
    let _ = app.run("insert.table", json!({"range": end, "header": true}));
    Ok(sheet)
}

/// A sheet name from the connector's name, without the characters a sheet
/// name cannot hold.
fn sheet_name(connector: &str) -> String {
    let base: String = connector.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect();
    let base: String = base.split_whitespace().collect::<Vec<_>>().join(" ");
    let base = base.trim().chars().take(31).collect::<String>();
    if base.is_empty() { "Connector".to_string() } else { base }
}

/// The letters for a column count: 1 → A, 26 → Z, 27 → AA.
fn column_letters(count: usize) -> String {
    let mut out = String::new();
    let mut n = count.max(1);
    while n > 0 {
        let rem = (n - 1) % 26;
        out.insert(0, (b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out
}

/// A connector from the command's parameters.
fn connector_from(p: &Json) -> Result<Connector, String> {
    let name = p.get("name").and_then(Json::as_str).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err("a connector needs a name".into());
    }
    let url = p.get("url").and_then(Json::as_str).unwrap_or("").trim().to_string();
    if url.is_empty() {
        return Err("a connector needs a URL".into());
    }
    let source = match p.get("source").and_then(Json::as_str).unwrap_or("rest") {
        "csv" => Source::Csv,
        "json" => Source::Json,
        _ => Source::Rest,
    };
    let method = p.get("method").and_then(Json::as_str).unwrap_or("GET").trim().to_uppercase();
    let path = p.get("path").and_then(Json::as_str).unwrap_or("").to_string();
    let append = p.get("append").and_then(Json::as_bool).unwrap_or(false);
    let headers = p
        .get("headers")
        .and_then(Json::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect::<Vec<_>>())
        .unwrap_or_default();
    // A connector made from a command's parameters is not one of the user's
    // saved connectors, so it cannot carry an `env:` header.
    Ok(Connector { name, url, source, method, headers, path, append, saved: false })
}

/// List the connectors the host offers.
pub fn list(app: &mut SheetApp) {
    let state = app.connector_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.job = Some(Box::new(|b: &dyn Backend| Outcome::List(b.list())));
}

/// Pull a connector's rows. The rows are written into a sheet when they
/// arrive.
pub fn fetch(app: &mut SheetApp, connector: Connector) {
    let state = app.connector_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.job = Some(Box::new(move |b: &dyn Backend| match b.fetch(&connector) {
        Ok(rows) => Outcome::Fetched { connector, rows },
        Err(e) => Outcome::Failed(e),
    }));
}

/// Save a connector, so it is offered next time.
pub fn save(app: &mut SheetApp, connector: Connector) {
    let state = app.connector_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.job = Some(Box::new(move |b: &dyn Backend| match b.save(&connector) {
        Ok(()) => Outcome::Saved(connector.name.clone()),
        Err(e) => Outcome::Failed(e),
    }));
}

/// Remove a connector by name.
pub fn remove(app: &mut SheetApp, name: String) {
    let state = app.connector_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.job = Some(Box::new(move |b: &dyn Backend| match b.remove(&name) {
        Ok(()) => Outcome::Saved("removed".into()),
        Err(e) => Outcome::Failed(e),
    }));
}

/// Pull a connector by name from the host's list.
pub fn fetch_named(app: &mut SheetApp, name: &str) {
    if app.services.connectors.is_empty() {
        app.message = Some((iridium_core::brand::NAME.into(), "connectors are not available in this build".into()));
        return;
    };
    let name = name.to_string();
    let state = app.connector_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.job = Some(Box::new(move |b: &dyn Backend| match b.list().into_iter().find(|c| c.name == name) {
        // It comes from the host's own list, so it is the user's saved
        // connector: only such a one may carry an `env:` header.
        Some(mut c) => {
            c.saved = true;
            match b.fetch(&c) {
                Ok(rows) => Outcome::Fetched { connector: c, rows },
                Err(e) => Outcome::Failed(e),
            }
        }
        None => Outcome::Failed(format!("there is no connector named {name}")),
    }));
}

/// Build a connector from the command's parameters, or fail with a readable
/// message.
pub fn connector_param(p: &Json) -> Result<Connector, String> {
    connector_from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_count() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(52), "AZ");
        assert_eq!(column_letters(53), "BA");
    }

    #[test]
    fn a_connector_needs_a_name_and_a_url() {
        assert!(connector_from(&json!({"url": "https://x"})).is_err());
        assert!(connector_from(&json!({"name": "x"})).is_err());
        assert!(connector_from(&json!({"name": "x", "url": "https://y"})).is_ok());
    }

    #[test]
    fn the_source_and_method_are_read() {
        let c = connector_from(&json!({"name": "x", "url": "https://y", "source": "csv", "method": "post"})).unwrap();
        assert_eq!(c.source, Source::Csv);
        assert_eq!(c.method, "POST");
    }

    #[test]
    fn headers_are_read_as_pairs() {
        let c = connector_from(&json!({"name": "x", "url": "https://y", "headers": {"Accept": "application/json"}})).unwrap();
        assert_eq!(c.headers, vec![("Accept".to_string(), "application/json".to_string())]);
    }
}
