//! Host-provided access to external services through a native connector.
//!
//! The connector is what makes a workbook talk to the world: a named source
//! (a REST endpoint, a CSV feed, a JSON API), the credentials it needs, and
//! the rows it returns. It is the same capability that an Excel add-in for
//! planning or an app-builder integration provides, built in rather than
//! installed.
//!
//! The interface is deliberately narrow. The host does the network I/O — the
//! app never blocks the UI thread on a socket — and returns plain rows, so
//! the engine can write them into a sheet without knowing where they came
//! from.
//!
//! The host is expected to run off the UI thread: `fetch` does network I/O,
//! and the UI calls it on a worker.

use std::fmt;

/// Where a connector's rows come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A REST endpoint returning JSON or CSV.
    #[default]
    Rest,
    /// A CSV feed.
    Csv,
    /// A JSON array of objects.
    Json,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::Rest => "rest",
            Source::Csv => "csv",
            Source::Json => "json",
        })
    }
}

/// One external source a workbook can pull from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connector {
    /// The name the user gave it, unique in the workbook.
    pub name: String,
    /// The endpoint or file the rows come from.
    pub url: String,
    /// What kind of source it is.
    pub source: Source,
    /// The HTTP method, for a REST source.
    pub method: String,
    /// Extra headers to send. A header whose value is `env:NAME@host` is
    /// replaced by that environment variable's value and sent only to
    /// `host`, so a secret stays out of the workbook and follows the host it
    /// was saved for.
    pub headers: Vec<(String, String)>,
    /// Where in the reply the rows are: a JSON path like `data.items`. Empty
    /// means the whole reply.
    pub path: String,
    /// Whether a refresh replaces the sheet's rows or adds to them.
    pub append: bool,
    /// Whether this is one of the user's own saved connectors. A saved
    /// connector is the only one whose `env:` headers are read, so a
    /// connector a caller made up cannot carry a secret away.
    pub saved: bool,
}

impl Connector {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Connector {
            name: name.into(),
            url: url.into(),
            source: Source::Rest,
            method: "GET".into(),
            headers: Vec::new(),
            path: String::new(),
            append: false,
            saved: false,
        }
    }
}

/// The rows a connector returned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    /// The column headings.
    pub header: Vec<String>,
    /// The rows, each as long as the header.
    pub rows: Vec<Vec<String>>,
    /// What the connector said about the fetch: a count, a note.
    pub note: String,
}

impl Rows {
    pub fn new(header: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Rows { header, rows, note: String::new() }
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The host's connector backend.
pub trait Backend: Send + Sync {
    /// The connectors the host offers, by name.
    fn list(&self) -> Vec<Connector>;

    /// Pull the rows a connector holds.
    ///
    /// The error text is shown to the user as-is, so it should say what went
    /// wrong and what to do, not name internal symbols.
    fn fetch(&self, connector: &Connector) -> Result<Rows, String>;

    /// Save a connector, so it is offered next time.
    fn save(&self, connector: &Connector) -> Result<(), String>;

    /// Remove a connector by name.
    fn remove(&self, name: &str) -> Result<(), String>;
}

/// The default host: no connectors. The UI reports the feature as
/// "not available in this build" rather than pretending it is broken.
#[derive(Debug, Default)]
pub struct NoneBackend;

impl Backend for NoneBackend {
    fn list(&self) -> Vec<Connector> {
        Vec::new()
    }
    fn fetch(&self, _connector: &Connector) -> Result<Rows, String> {
        Err("Connectors are not available in this build.".into())
    }
    fn save(&self, _connector: &Connector) -> Result<(), String> {
        Err("Connectors are not available in this build.".into())
    }
    fn remove(&self, _name: &str) -> Result<(), String> {
        Err("Connectors are not available in this build.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connector_defaults_to_a_rest_get() {
        let c = Connector::new("x", "https://example.invalid");
        assert_eq!(c.source, Source::Rest);
        assert_eq!(c.method, "GET");
        assert!(c.headers.is_empty());
        assert!(c.path.is_empty());
        assert!(!c.append);
    }

    #[test]
    fn the_source_is_named_as_the_helper_names_it() {
        assert_eq!(Source::Rest.to_string(), "rest");
        assert_eq!(Source::Csv.to_string(), "csv");
        assert_eq!(Source::Json.to_string(), "json");
    }

    #[test]
    fn rows_count_what_they_hold() {
        let r = Rows::new(vec!["a".into()], vec![vec!["1".into()]]);
        assert_eq!(r.len(), 1);
        assert!(!r.is_empty());
        assert!(Rows::default().is_empty());
    }
}
