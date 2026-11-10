//! The desktop shell's connector backend.
//!
//! The host does the network I/O so the UI thread never waits on a socket.
//! The work happens in a small Python helper — the same arrangement the cloud
//! spreadsheet backends use — which keeps the app free of an HTTP stack and
//! keeps the request handling in one readable place.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use iridium_ui_egui::services::connector::{Backend, Connector, Rows, Source};

/// The helper, baked into the binary so it works from any install directory.
const SCRIPT: &str = include_str!("../../../tools/connector_backend.py");

pub struct DesktopBackend {
    /// The helper's path, once it has been found or written.
    script: Mutex<Option<PathBuf>>,
}

impl DesktopBackend {
    pub fn new() -> Self {
        DesktopBackend { script: Mutex::new(None) }
    }

    /// The helper's path.
    ///
    /// A packaged build ships it inside the app bundle (macOS:
    /// `Contents/Resources/pylib`, Windows: `pylib` beside the executable), so
    /// look there first. A development checkout has it in the source tree, so
    /// the embedded copy is written to a temp file and used instead.
    fn script_path(&self) -> Result<PathBuf, String> {
        let mut slot = self.script.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(p) = slot.as_ref()
            && p.exists()
        {
            return Ok(p.clone());
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let candidates = [
                dir.join("connector_backend.py"),
                dir.join("pylib").join("connector_backend.py"),
                dir.join("..").join("Resources").join("pylib").join("connector_backend.py"),
            ];
            for c in candidates {
                if c.exists() {
                    *slot = Some(c.clone());
                    return Ok(c);
                }
            }
        }
        // The helper goes in a directory only the user may enter, not in the
        // shared temp directory: Python puts the script's own directory first
        // on its import path, so a helper in /tmp would import /tmp/json.py
        // planted by another account.
        let dir = private_dir()?;
        let path = dir.join("connector_backend.py");
        write_private(&path, SCRIPT).map_err(|e| format!("could not write the connector helper: {e}"))?;
        *slot = Some(path.clone());
        Ok(path)
    }

    /// Run one request against the helper and return its JSON response.
    fn call(&self, request: serde_json::Value) -> Result<serde_json::Value, String> {
        let script = self.script_path()?;
        let mut child = Command::new(python())
            // `-I` keeps Python from the environment: no PYTHONPATH, no user
            // site directory, no current directory on the import path.
            .arg("-I")
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run the connector helper: {e}"))?;
        {
            let stdin = child.stdin.as_mut().ok_or("the connector helper closed its input")?;
            stdin.write_all(request.to_string().as_bytes()).map_err(|e| format!("could not talk to the connector helper: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("the connector helper failed: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let response: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).map(str::trim).unwrap_or("");
            if detail.is_empty() {
                format!("the connector helper said something unreadable: {e}")
            } else {
                format!("the connector helper failed: {detail}")
            }
        })?;
        if response.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            Ok(response)
        } else {
            Err(response.get("error").and_then(serde_json::Value::as_str).unwrap_or("the connector failed").to_string())
        }
    }
}

impl Default for DesktopBackend {
    fn default() -> Self {
        DesktopBackend::new()
    }
}

impl Backend for DesktopBackend {
    fn list(&self) -> Vec<Connector> {
        let Ok(r) = self.call(serde_json::json!({"op": "list"})) else {
            return Vec::new();
        };
        r.get("connectors")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.iter().filter_map(connector_from_json).collect())
            .unwrap_or_default()
    }

    fn fetch(&self, connector: &Connector) -> Result<Rows, String> {
        let r = self.call(serde_json::json!({"op": "fetch", "connector": connector_to_json(connector)}))?;
        let header = r
            .get("header")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect())
            .unwrap_or_default();
        let rows = r
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|row| {
                        row.as_array().map(|cells| cells.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let note = r.get("note").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        Ok(Rows { header, rows, note })
    }

    fn save(&self, connector: &Connector) -> Result<(), String> {
        self.call(serde_json::json!({"op": "save", "connector": connector_to_json(connector)}))?;
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        self.call(serde_json::json!({"op": "remove", "name": name}))?;
        Ok(())
    }
}

/// A connector as the helper sends it.
fn connector_to_json(c: &Connector) -> serde_json::Value {
    serde_json::json!({
        "name": c.name,
        "url": c.url,
        "source": c.source.to_string(),
        "method": c.method,
        "headers": c.headers.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>(),
        "path": c.path,
        "append": c.append,
        "saved": c.saved,
    })
}

/// A connector as the helper sends it, read back.
fn connector_from_json(v: &serde_json::Value) -> Option<Connector> {
    let name = v.get("name")?.as_str()?.to_string();
    let url = v.get("url").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let source = match v.get("source").and_then(serde_json::Value::as_str).unwrap_or("rest") {
        "csv" => Source::Csv,
        "json" => Source::Json,
        _ => Source::Rest,
    };
    let method = v.get("method").and_then(serde_json::Value::as_str).unwrap_or("GET").to_string();
    let headers = v
        .get("headers")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|p| {
                    let pair = p.as_array()?;
                    Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let path = v.get("path").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let append = v.get("append").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let saved = v.get("saved").and_then(serde_json::Value::as_bool).unwrap_or(false);
    Some(Connector { name, url, source, method, headers, path, append, saved })
}

/// The Python to run the helper with.
///
/// A candidate is only accepted once it has actually run: on macOS
/// `/usr/bin/python3` can be the Xcode stub, which exists but answers with
/// an error and no output.
fn python() -> PathBuf {
    if let Ok(p) = std::env::var("IRIDIUM_PYTHON") {
        return PathBuf::from(p);
    }
    let mut candidates: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        candidates.push(PathBuf::from("py"));
        candidates.push(PathBuf::from("python.exe"));
        candidates.push(PathBuf::from("python3.exe"));
    } else {
        candidates.push(PathBuf::from("/usr/bin/python3"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/python3"));
        candidates.push(PathBuf::from("python3"));
    }
    for c in &candidates {
        if runs(c) {
            return c.clone();
        }
    }
    candidates.remove(0)
}

/// Whether this candidate really is a working Python.
fn runs(candidate: &PathBuf) -> bool {
    Command::new(candidate)
        .arg("-c")
        .arg("import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A directory only this user may enter, for the helper that is written out.
///
/// The name has a random part: another account must not be able to guess it
/// and create it first, which would put the helper in a directory they own.
fn private_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    for attempt in 0..8 {
        // The clock comes from the engine, the way the rest of the app reads
        // it: `SystemTime::now` traps on wasm.
        let stamp = iridium_engine::core::date::unix_now_ms().unwrap_or_default();
        let dir = base.join(format!("iridium-sheets-{}-{stamp}-{attempt}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                set_private(&dir);
                return Ok(dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not make a private directory for the connector helper: {e}")),
        }
    }
    Err("could not make a private directory for the connector helper".into())
}

/// Restrict a path to this user.
#[cfg(unix)]
fn set_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private(_path: &std::path::Path) {}

/// Write a file that only the user can read.
#[cfg(unix)]
fn write_private(path: &PathBuf, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &PathBuf, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}
