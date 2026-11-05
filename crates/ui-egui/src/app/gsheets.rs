//! Cloud spreadsheet commands: open a spreadsheet from Google Sheets or
//! Excel, and save the current workbook back.
//!
//! The host does the network work (`services::gsheets::Backend`) so the UI
//! never blocks on it: the calls run on a worker thread and the result is
//! collected on the next frame.

use super::super::services::gsheets::{Account, Backend, Provider, SheetInfo};
use super::super::{Json, Notice, NoticeKind, SheetApp, clean_error, json};
use std::sync::{Arc, Mutex};

/// What a finished host call produced. The worker hands this back; the UI
/// reads it on the next frame.
#[derive(Debug)]
pub enum Outcome {
    /// The spreadsheet list for one provider.
    List { provider: Provider, sheets: Vec<SheetInfo> },
    /// A downloaded workbook, ready to open.
    Fetched { id: String, title: String, provider: Provider, bytes: Vec<u8> },
    /// An upload that finished.
    Pushed { id: String, title: String, provider: Provider },
    /// The signed-in account of one provider.
    Account { provider: Provider, account: Account },
    /// A sign-in URL for the user to visit.
    SignIn { provider: Provider, url: String },
    /// The call failed; the text is shown to the user.
    Failed { provider: Provider, message: String },
}

/// Work the host thread does, so the UI thread never waits on the network.
type Job = Box<dyn FnOnce(&dyn Backend) -> Outcome + Send + 'static>;

/// A job and the provider it is for.
///
/// The provider is part of the job because the host offers one backend per
/// provider, and the worker has to pick the right one: running an Excel job
/// against the Google backend asked Google's helper for an op it does not
/// have.
struct Task {
    provider: Provider,
    job: Job,
}

/// A pending job and its result.
#[derive(Default)]
pub struct Pending {
    task: Option<Task>,
    result: Option<Outcome>,
}

/// The pending cloud work, if any.
///
/// An `Arc<Mutex<..>>` rather than a bare `Mutex`: the worker thread needs
/// its own handle, and a bare mutex would keep a borrow of the app alive
/// across the mutable use that follows.
pub type State = Arc<Mutex<Pending>>;

/// Run the pending job on a worker and collect its result.
///
/// The worker owns a clone of the backend and the shared state; it never
/// touches the app, so the UI thread is never blocked on the network.
pub fn poll(app: &mut SheetApp) {
    let state = app.gsheets_state.clone();
    let task = {
        let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        s.task.take()
    };
    if let Some(Task { provider, job }) = task {
        let backend = app.services.gsheets.iter().find(|b| b.provider() == provider).cloned();
        let state_for_worker = state.clone();
        std::thread::spawn(move || {
            let out = match backend {
                Some(b) => job(b.as_ref()),
                None => Outcome::Failed { provider, message: "the cloud service is not available in this build".into() },
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
        Outcome::List { provider, sheets } => {
            app.gsheets_sheets.retain(|s| s.provider != provider);
            app.gsheets_sheets.extend(sheets);
        }
        Outcome::Account { provider, account } => {
            if provider == Provider::Google {
                app.gsheets_account = account;
            } else {
                app.gsheets_excel_account = account;
            }
        }
        Outcome::SignIn { provider, url } => {
            // An empty URL means the sign-in is done; otherwise the code goes
            // in the notice, because the user has to type it.
            if !url.is_empty()
                && let Some(open) = &app.services.open_url
            {
                open(&url);
            }
            app.notice = Some(Notice {
                text: if url.is_empty() {
                    format!("Signed in to {}.", provider.label())
                } else {
                    format!("Finish signing in to {} in your browser, then press Sign in again.\n\n{}", provider.label(), url)
                },
                kind: NoticeKind::Info,
                action: None,
            });
        }
        Outcome::Fetched { id, title, provider, bytes } => {
            let b64 = iridium_engine::io::base64_encode(&bytes);
            let name = format!("{}.xlsx", title);
            match app.run("file.open", json!({"name": name, "base64": b64})) {
                Ok(r) => {
                    app.after_open(None, &r);
                    app.gsheets_current = Some((provider, id));
                    app.notice = Some(Notice { text: format!("Opened {title} from {}.", provider.label()), kind: NoticeKind::Info, action: None });
                }
                Err(e) => app.message = Some((iridium_core::brand::NAME.into(), clean_error(&e))),
            }
            app.after_engine();
        }
        Outcome::Pushed { id, title, provider } => {
            app.gsheets_current = Some((provider, id));
            app.notice = Some(Notice { text: format!("Saved {title} to {}.", provider.label()), kind: NoticeKind::Info, action: None });
        }
        Outcome::Failed { provider, message } => {
            app.message = Some((iridium_core::brand::NAME.into(), clean_error(&format!("{}: {message}", provider.label()))));
        }
    }
}

/// Set a job, replacing any job still running: the older one's result is
/// dropped, which is what the user expects when they pick something else.
fn set_job(app: &SheetApp, provider: Provider, job: Job) {
    let state = app.gsheets_state.clone();
    let mut s = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    s.task = Some(Task { provider, job });
}

/// The backend for a provider, if the host offers one.
fn backend_for(app: &SheetApp, provider: Provider) -> Option<Arc<dyn Backend>> {
    app.services.gsheets.iter().find(|b| b.provider() == provider).cloned()
}

/// Start listing the spreadsheets the account can open.
pub fn list(app: &mut SheetApp, provider: Provider) {
    if backend_for(app, provider).is_none() {
        app.message = Some((iridium_core::brand::NAME.into(), format!("{} is not available in this build.", provider.label())));
        return;
    }
    set_job(
        app,
        provider,
        Box::new(move |b: &dyn Backend| match b.list() {
            Ok(v) => Outcome::List { provider, sheets: v },
            Err(e) => Outcome::Failed { provider, message: e },
        }),
    );
}

/// Start downloading a spreadsheet.
pub fn fetch(app: &mut SheetApp, provider: Provider, id: &str) {
    if backend_for(app, provider).is_none() {
        return;
    }
    let id = id.to_string();
    set_job(
        app,
        provider,
        Box::new(move |b: &dyn Backend| match b.fetch(&id) {
            Ok(bytes) => {
                let title =
                    b.list().ok().and_then(|v| v.into_iter().find(|s| s.id == id)).map(|s| s.title).unwrap_or_else(|| "Cloud spreadsheet".into());
                Outcome::Fetched { id, title, provider, bytes }
            }
            Err(e) => Outcome::Failed { provider, message: e },
        }),
    );
}

/// Start uploading the active workbook.
pub fn push(app: &mut SheetApp, provider: Provider, id: &str, tab: Option<&str>) {
    if backend_for(app, provider).is_none() {
        return;
    }
    let id = id.to_string();
    let tab = tab.map(str::to_string);
    // Encode the workbook first: the engine call is cheap and local.
    let bytes = match app.session.run("file.saveBytes", json!({"format": "xlsx"})) {
        Ok(r) => r.get("base64").and_then(Json::as_str).and_then(iridium_engine::io::base64_decode),
        Err(e) => {
            app.message = Some((iridium_core::brand::NAME.into(), clean_error(&e)));
            return;
        }
    };
    let Some(bytes) = bytes else {
        app.message = Some((iridium_core::brand::NAME.into(), "Could not encode the workbook.".into()));
        return;
    };
    set_job(
        app,
        provider,
        Box::new(move |b: &dyn Backend| match b.push(&id, &bytes, tab.as_deref()) {
            Ok(title) => Outcome::Pushed { id, title, provider },
            Err(e) => Outcome::Failed { provider, message: e },
        }),
    );
}

/// The sign-in URL, so the UI can offer "Sign in".
pub fn sign_in(app: &mut SheetApp, provider: Provider) {
    if backend_for(app, provider).is_none() {
        return;
    }
    set_job(
        app,
        provider,
        Box::new(move |b: &dyn Backend| match b.sign_in_url() {
            Ok(url) => Outcome::SignIn { provider, url },
            Err(e) => Outcome::Failed { provider, message: e },
        }),
    );
}

/// Ask the host which account is signed in.
pub fn account(app: &mut SheetApp, provider: Provider) {
    if backend_for(app, provider).is_none() {
        return;
    }
    set_job(app, provider, Box::new(move |b: &dyn Backend| Outcome::Account { provider, account: b.account() }));
}

#[cfg(test)]
#[path = "gsheets_tests.rs"]
mod gsheets_tests;
