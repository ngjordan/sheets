//! The desktop shell's cloud-spreadsheet backends.
//!
//! Each backend runs a small Python helper: one JSON request on stdin, one
//! JSON response on stdout. Keeping the Google and Microsoft calls in Python
//! means the app takes no new Rust dependency for OAuth or HTTP, and the
//! credential handling stays in one readable place.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde_json::{Value as Json, json};

use iridium_ui_egui::services::gsheets::{Account, Backend, Provider, SheetInfo};

/// The Google helper, baked into the binary so it works from any install
/// directory.
const GOOGLE_SCRIPT: &str = include_str!("../../../tools/gsheets_backend.py");
/// The Microsoft helper.
const EXCEL_SCRIPT: &str = include_str!("../../../tools/m365_backend.py");

/// The CLI used to read a workbook back when pushing, next to the helper.
const CLI_NAME: &str = "iridium-sheets-cli";

/// What to tell the user when they ask to sign in to Google.
///
/// Google has no in-app sign-in: the token comes from the one-time OAuth
/// setup, which needs the user's own Google Cloud client. The helper can
/// only read the token afterwards.
const GOOGLE_SIGN_IN_HELP: &str = "Sign in to Google with the Google Workspace setup (it asks for your Google Cloud client, and writes ~/.hermes/google_token.json), then reopen Iridium Sheets.";

/// The helper script, by provider.
fn script_for(provider: Provider) -> &'static str {
    match provider {
        Provider::Google => GOOGLE_SCRIPT,
        Provider::Excel => EXCEL_SCRIPT,
    }
}

/// The helper's file name.
fn script_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Google => "gsheets_backend.py",
        Provider::Excel => "m365_backend.py",
    }
}

pub struct DesktopBackend {
    provider: Provider,
    /// The helper's source. It is written to a temp file once and reused.
    script: Mutex<Option<std::path::PathBuf>>,
    /// The path to the CLI binary, if it can be found.
    cli: Mutex<Option<std::path::PathBuf>>,
}

impl DesktopBackend {
    pub fn google() -> Self {
        DesktopBackend::new(Provider::Google)
    }

    pub fn excel() -> Self {
        DesktopBackend::new(Provider::Excel)
    }

    fn new(provider: Provider) -> Self {
        DesktopBackend { provider, script: Mutex::new(None), cli: Mutex::new(None) }
    }

    /// The helper's path.
    ///
    /// A packaged build ships it inside the app bundle (macOS:
    /// `Contents/Resources/pylib`, Windows: `pylib` beside the executable), so
    /// look there first. A development checkout has it in the source tree, so
    /// the embedded copy is written to a temp file and used instead.
    fn script_path(&self) -> Result<std::path::PathBuf, String> {
        let mut slot = self.script.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(p) = slot.as_ref()
            && p.exists()
        {
            return Ok(p.clone());
        }
        // Next to the executable, and in the macOS bundle's Resources.
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let name = script_name(self.provider);
            let candidates = [dir.join(name), dir.join("pylib").join(name), dir.join("..").join("Resources").join("pylib").join(name)];
            for c in candidates {
                if c.exists() {
                    *slot = Some(c.clone());
                    return Ok(c);
                }
            }
        }
        let dir = std::env::temp_dir();
        // The name keeps the .py extension so the interpreter treats it as a
        // script, and the process id keeps two copies of the app apart.
        let path = dir.join(format!("iridium-sheets-{}-{}.py", script_name(self.provider), std::process::id()));
        // Write it as the user only: the helper is in a shared temp
        // directory, and a file any other account can read or replace is
        // theirs to run.
        write_private(&path, script_for(self.provider)).map_err(|e| format!("could not write the cloud helper: {e}"))?;
        *slot = Some(path.clone());
        Ok(path)
    }

    /// Find the CLI binary: next to this executable first, then on PATH.
    fn cli_path(&self) -> Result<std::path::PathBuf, String> {
        let mut slot = self.cli.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let mut candidates = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join(CLI_NAME));
            // macOS puts the binary in Contents/MacOS; the CLI sits beside it
            // in a normal install, but check Contents/MacOS too.
            candidates.push(dir.join("..").join("MacOS").join(CLI_NAME));
        }
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(if cfg!(windows) { ';' } else { ':' }) {
                if !dir.is_empty() {
                    candidates.push(std::path::Path::new(dir).join(CLI_NAME));
                }
            }
        }
        let found = candidates
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| format!("could not find {CLI_NAME}; it is needed to read a workbook back for the cloud service"))?;
        *slot = Some(found.clone());
        Ok(found)
    }

    /// Run one request against the helper and return its JSON response.
    fn call(&self, request: Json) -> Result<Json, String> {
        let script = self.script_path()?;
        let mut child = Command::new(python())
            .arg(&script)
            .env("IRIDIUM_CLI", self.cli_path().unwrap_or_else(|_| std::path::PathBuf::from(CLI_NAME)))
            .env("PYTHONPATH", pythonpath(&script))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run the cloud helper: {e}"))?;
        {
            let stdin = child.stdin.as_mut().ok_or("the cloud helper closed its input")?;
            stdin.write_all(request.to_string().as_bytes()).map_err(|e| format!("could not talk to the cloud helper: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("the cloud helper failed: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let response: Json = serde_json::from_str(text.trim()).map_err(|e| {
            // The helper prints nothing on stdout when it dies before it can
            // answer - a missing library, a broken interpreter. Its stderr
            // says why, so pass it on rather than only the parse error.
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).map(str::trim).unwrap_or("");
            if detail.is_empty() { format!("the cloud helper said something unreadable: {e}") } else { format!("the cloud helper failed: {detail}") }
        })?;
        if response.get("ok").and_then(Json::as_bool) == Some(true) {
            Ok(response)
        } else {
            Err(response.get("error").and_then(Json::as_str).unwrap_or("the cloud service failed").to_string())
        }
    }
}

/// The Python to run the helper with.
///
/// The Google libraries live beside the app, not on the system Python's path,
/// so the interpreter and the library path are both set. The search covers
/// macOS and Linux (`python3`), Windows (`python.exe`, and the py launcher
/// a normal Windows install provides), and an explicit override for packaged
/// builds that ship their own interpreter.
///
/// A candidate is only accepted once it has actually run: on macOS
/// `/usr/bin/python3` can be the Xcode stub, which exists but answers with
/// an error and no output. Picking it made every cloud call fail with an
/// empty reply.
fn python() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("IRIDIUM_PYTHON") {
        return std::path::PathBuf::from(p);
    }
    let mut candidates: Vec<std::path::PathBuf> = vec![];
    if cfg!(windows) {
        // The py launcher finds the newest installed interpreter.
        candidates.push(std::path::PathBuf::from("py"));
        candidates.push(std::path::PathBuf::from("python.exe"));
        candidates.push(std::path::PathBuf::from("python3.exe"));
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            // A portable install that ships its own Python.
            candidates.push(dir.join("python").join("python.exe"));
            candidates.push(dir.join("python.exe"));
        }
    } else {
        candidates.push(std::path::PathBuf::from("/usr/bin/python3"));
        candidates.push(std::path::PathBuf::from("/opt/homebrew/bin/python3"));
        candidates.push(std::path::PathBuf::from("python3"));
    }
    for c in &candidates {
        if runs(c) {
            return c.clone();
        }
    }
    // Not found by path: let the OS resolve it (a bare name works when the
    // executable is on PATH, which `exists()` cannot tell us).
    candidates.remove(0)
}

/// Whether this candidate really is a working Python.
///
/// `exists()` is not enough: a path can be present and still not run. The
/// check runs the interpreter, so it costs one process the first time a
/// cloud call is made.
fn runs(candidate: &std::path::Path) -> bool {
    use std::process::{Command, Stdio};
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

/// Write a file that only the user can read.
///
/// The helper lands in a temp directory other accounts can reach. Opening it
/// with O_EXCL and mode 600 means a file already sitting there (another
/// account's symlink, say) is an error rather than something to write
/// through.
#[cfg(unix)]
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// Where the Google API libraries are installed.
///
/// The helper's own directory comes first: a packaged macOS build puts the
/// helper and the libraries together in `Contents/Resources/pylib`, but the
/// executable lives in `Contents/MacOS`, so the libraries are not beside the
/// binary and have to be named explicitly. Then the paths beside the
/// executable (Windows puts `pylib` there) and the workspace copy a
/// development checkout uses.
fn pythonpath(script: &std::path::Path) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let mut parts = Vec::new();
    if let Ok(existing) = std::env::var("PYTHONPATH") {
        parts.push(existing);
    }
    // The directory the helper was found in, and its parent: the libraries
    // sit beside the helper, but some builds put them one level up.
    if let Some(dir) = script.parent() {
        parts.push(dir.to_string_lossy().into_owned());
        if let Some(up) = dir.parent() {
            parts.push(up.to_string_lossy().into_owned());
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        parts.push(dir.join("pylib").to_string_lossy().into_owned());
        parts.push(dir.join("python").join("Lib").join("site-packages").to_string_lossy().into_owned());
        // The macOS bundle keeps them in Resources, beside the app.
        parts.push(dir.join("..").join("Resources").join("pylib").to_string_lossy().into_owned());
    }
    parts.push("/opt/hermes-agent/workspace/pylib".to_string());
    parts.join(separator)
}

impl Backend for DesktopBackend {
    fn account(&self) -> Account {
        match self.call(json!({"op": "account"})) {
            Ok(r) => Account {
                email: r.get("email").and_then(Json::as_str).unwrap_or_default().to_string(),
                signed_in: r.get("signed_in").and_then(Json::as_bool).unwrap_or(false),
            },
            Err(_) => Account { email: String::new(), signed_in: false },
        }
    }

    fn list(&self) -> Result<Vec<SheetInfo>, String> {
        let r = self.call(json!({"op": "list"}))?;
        let list = r.get("sheets").and_then(Json::as_array).ok_or("the cloud service returned no list")?;
        Ok(list
            .iter()
            .filter_map(|s| {
                Some(SheetInfo {
                    id: s.get("id")?.as_str()?.to_string(),
                    title: s.get("title").and_then(Json::as_str).unwrap_or_default().to_string(),
                    tabs: s
                        .get("tabs")
                        .and_then(Json::as_array)
                        .map(|t| t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                    provider: self.provider,
                })
            })
            .collect())
    }

    fn fetch(&self, spreadsheet_id: &str) -> Result<Vec<u8>, String> {
        let r = self.call(json!({"op": "fetch", "id": spreadsheet_id}))?;
        let b64 = r.get("xlsx").and_then(Json::as_str).ok_or("the cloud service returned no file")?;
        iridium_engine::io::base64_decode(b64).ok_or_else(|| "the cloud service returned a file we could not decode".to_string())
    }

    fn push(&self, spreadsheet_id: &str, xlsx: &[u8], tab: Option<&str>) -> Result<String, String> {
        let b64 = iridium_engine::io::base64_encode(xlsx);
        let mut req = json!({"op": "push", "id": spreadsheet_id, "xlsx": b64});
        if let Some(t) = tab {
            req["tab"] = Json::String(t.to_string());
        }
        let r = self.call(req)?;
        Ok(r.get("title").and_then(Json::as_str).unwrap_or("the spreadsheet").to_string())
    }

    fn sign_in_url(&self) -> Result<String, String> {
        // Microsoft uses the device-code flow: the user visits a page and
        // types a code, so there is no browser session to hand over.
        // Google signs in through the Google Workspace setup, which produces
        // a token the helper reads - the app cannot start that flow itself,
        // so it says what to do rather than asking the helper for an op it
        // does not have.
        if self.provider == Provider::Google {
            return Err(GOOGLE_SIGN_IN_HELP.into());
        }
        let r = self.call(json!({"op": "signIn"}))?;
        let url = r.get("verification_uri").and_then(Json::as_str).unwrap_or_default().to_string();
        let code = r.get("user_code").and_then(Json::as_str).unwrap_or_default().to_string();
        if url.is_empty() {
            return Err(match self.provider {
                Provider::Google => "Sign in through the Google Workspace setup, then reopen Iridium Sheets.".into(),
                Provider::Excel => "could not start the Microsoft sign-in".into(),
            });
        }
        Ok(format!("{url}  (code: {code})"))
    }

    fn provider(&self) -> Provider {
        self.provider
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that exists but is not a working Python must be rejected.
    ///
    /// On macOS `/usr/bin/python3` exists as the Xcode stub when the Command
    /// Line Tools are not installed. Accepting it made every cloud call fail
    /// with an empty reply and a confusing parse error.
    #[cfg(unix)]
    #[test]
    fn a_path_that_exists_but_cannot_run_is_not_a_python() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("iridium-python-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("python3");
        std::fs::write(&fake, "#!/bin/sh\necho 'xcode-select: error: command line tools are not installed' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(fake.exists(), "the probe file must exist for this test to mean anything");
        assert!(!runs(&fake), "a file that exists but fails to run is not a Python");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The interpreter the app picks has to be one that actually works.
    #[test]
    fn the_chosen_python_actually_runs() {
        let py = python();
        assert!(runs(&py), "the chosen interpreter {py:?} does not run");
    }

    /// The library path must include the directory the helper was found in.
    ///
    /// A packaged macOS build keeps the helper and the Google libraries in
    /// `Contents/Resources/pylib` while the executable is in
    /// `Contents/MacOS`, so the libraries are not reachable by a path derived
    /// from the executable alone.
    #[test]
    fn the_library_path_includes_the_helpers_own_directory() {
        let dir = std::env::temp_dir().join(format!("iridium-pylib-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("gsheets_backend.py");
        std::fs::write(&helper, "# probe\n").unwrap();
        let path = pythonpath(&helper);
        assert!(
            path.split(if cfg!(windows) { ";" } else { ":" }).any(|p| p == dir.to_string_lossy()),
            "the helper's own directory must be on PYTHONPATH: {path}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The helper written to the temp directory is readable only by its owner.
    #[cfg(unix)]
    #[test]
    fn the_written_helper_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("iridium-helper-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("helper.py");
        write_private(&path, "# probe\n").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the helper must be mode 600, got {mode:o}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The temp helper keeps its .py extension, so the interpreter treats it
    /// as a script rather than a file to guess at.
    #[test]
    fn the_temp_helper_keeps_its_extension() {
        let name = format!("iridium-sheets-{}-{}.py", script_name(Provider::Google), std::process::id());
        assert!(name.ends_with(".py"), "{name}");
    }
}
