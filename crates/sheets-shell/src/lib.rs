//! Sheets desktop shell: everything the Sheets window does except the process-wide set-up that
//! stays in `apps/iridium-sheets` (logging, graphics backend, updater, macOS process exit).
//!
//! The binary builds one [`Shell`] and forwards eframe's callbacks to it.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "macos")]
mod apple_events;
mod connector;
mod control_server;
mod gsheets;
#[cfg(target_os = "macos")]
mod native_menu;
mod print;

use std::cell::RefCell;
use std::rc::Rc;

use iridium_engine::Session;
use iridium_ui_egui::doc_windows::DocWindows;
use iridium_ui_egui::{Services, SheetApp};
use serde_json::json;

/// What the app was started with, parsed by the binary.
#[derive(Clone)]
pub struct Launch {
    /// Files named on the command line, opened in order.
    pub files: Vec<String>,
    /// `--sample <name>`: start with this sample workbook.
    pub sample: Option<String>,
    /// `--control <port>` / `IRIDIUM_CONTROL_PORT`.
    pub control_port: Option<u16>,
    /// `--control-token` / `IRIDIUM_CONTROL_TOKEN`.
    pub control_token: Option<String>,
    /// The window's viewport; each workbook window is built from it.
    pub viewport: egui::ViewportBuilder,
    /// False under `IRIDIUM_NO_PREFS`: preferences are neither read nor written.
    pub prefs: bool,
    /// The settings folder holding `ui.json` and `prefs.json`.
    pub config_dir: Option<std::path::PathBuf>,
    /// macOS: the Apple-event queue the binary registered before the event loop started.
    #[cfg(target_os = "macos")]
    pub inbox: fmv_macos_events::Inbox,
}

/// The app, its native window per workbook window, and (macOS) the menu bar and Finder events.
pub struct Shell {
    app: SheetApp,
    windows: DocWindows,
    launch: Launch,
    #[cfg(target_os = "macos")]
    menu: RefCell<Option<native_menu::NativeMenu>>,
    #[cfg(target_os = "macos")]
    inbox: fmv_macos_events::Inbox,
    #[cfg(target_os = "macos")]
    ctx: egui::Context,
    fonts: egui::FontDefinitions,
    style: egui::Style,
    zoom: f32,
}

impl Shell {
    /// Build the shell: the workbook session, preferences, command-line files, the control
    /// channel and the workbook windows.
    ///
    /// A thin wrapper over [`Shell::new_for_host`]; Sheets needs nothing from `cc` beyond its
    /// context and storage (which it ignores).
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        Self::new_for_host(&cc.egui_ctx, cc.storage, launch)
    }

    /// Host-facing constructor: no `CreationContext`, so a host can build this module later.
    /// `storage` is unused (Sheets keeps its settings in `launch.config_dir`). The fonts, style and
    /// zoom Sheets installed into `ctx` are captured for [`fonts`](Self::fonts),
    /// [`style`](Self::style) and [`zoom`](Self::zoom).
    pub fn new_for_host(ctx: &egui::Context, _storage: Option<&dyn eframe::Storage>, launch: Launch) -> Self {
        let mut session = Session::new();
        if let Some(s) = &launch.sample
            && let Err(e) = session.execute("file.new", json!({"sample": s}))
        {
            log::warn!("sample {s}: {e}");
        }
        let mut app = SheetApp::new(session, services());
        app.watch_desktop_theme();
        app.start_update_check();
        load_prefs(&mut app, &launch);
        // Stamp the startup workbook's author from the persisted user name.
        if app.session.documents().is_empty() {
            app.session.new_workbook();
        }
        app.after_engine(); // Show warnings from the sample.
        // Files named on the command line (a double-click on Windows and Linux) open as File › Open
        // opens them: a password-protected one asks for its password, and errors are shown, not
        // only logged. Each replaces the untouched blank Book1, as a file opened from Finder does.
        for f in &launch.files {
            app.open_path(f);
        }
        SheetApp::setup_context_for_language(ctx, app.ui.dark, app.ui.language);
        // Capture the fonts Sheets installed. `Context::fonts` PANICS before egui's first pass
        // ("No fonts available until first call to Context::run()"), and a host may build this
        // module before any frame has run, so capture the definitions the app built instead of
        // reading them back.
        let fonts = iridium_ui_egui::theme::font_definitions_for_language(app.ui.language);
        if let Some(port) = launch.control_port {
            match control_server::start(port, launch.control_token.clone(), ctx.clone()) {
                Ok(rx) => app.control_rx = Some(rx),
                Err(e) => log::error!("control channel on port {port}: {e}"),
            }
        }
        let windows = DocWindows::new(&mut app, launch.viewport.clone(), DocWindows::platform_quits_with_last_window());
        #[cfg(target_os = "macos")]
        let inbox = {
            let ctx = ctx.clone();
            launch.inbox.set_wake(move || ctx.request_repaint());
            launch.inbox.clone()
        };
        Self {
            app,
            windows,
            launch,
            #[cfg(target_os = "macos")]
            menu: RefCell::new(None),
            #[cfg(target_os = "macos")]
            inbox,
            #[cfg(target_os = "macos")]
            ctx: ctx.clone(),
            fonts,
            style: ctx.global_style().as_ref().clone(),
            zoom: ctx.zoom_factor(),
        }
    }

    /// The font set Sheets installed, captured at construction.
    pub fn fonts(&self) -> egui::FontDefinitions {
        self.fonts.clone()
    }

    /// The style Sheets left behind, captured at construction.
    pub fn style(&self) -> egui::Style {
        self.style.clone()
    }

    /// The zoom factor Sheets left behind, captured at construction.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// macOS: re-install Sheets's native menu as the menu bar. No-op elsewhere.
    pub fn install_native_menu(&self) {
        #[cfg(target_os = "macos")]
        if std::env::var_os("IRIDIUM_NO_NATIVE_MENU").is_none()
            && let Ok(mut menu) = self.menu.try_borrow_mut()
        {
            *menu = Some(native_menu::NativeMenu::install(&self.ctx));
        }
    }

    /// Per-frame logic, called before `ui`.
    pub fn logic(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        {
            if self.menu.get_mut().is_none() && std::env::var_os("IRIDIUM_NO_NATIVE_MENU").is_none() {
                *self.menu.get_mut() = Some(native_menu::NativeMenu::install(ctx));
            }
            if let Some(m) = self.menu.get_mut() {
                m.poll(&mut self.app, ctx, self.windows.focused_viewport());
            }
            apple_events::poll(&self.inbox, &mut self.app, ctx);
        }
        self.app.logic(ctx);
        self.windows.logic(&mut self.app, ctx);
        if ctx.input(|input| input.viewport().close_requested()) && self.app.printing.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Forwards raw input to the app, then opens every non-empty dropped file path as a workbook.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
        // Files dropped on the window open as workbooks.
        for f in std::mem::take(&mut raw.dropped_files) {
            if let Some(p) = f.path().to_str().map(str::to_string).filter(|p| !p.is_empty()) {
                self.open_path(&p);
            }
        }
    }

    /// Draw the app into the given region.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.windows.show(&mut self.app, ui);
    }

    /// Open a file path as a workbook.
    pub fn open_path(&mut self, path: &str) {
        self.app.open_path(path);
    }

    /// Waits for print jobs to reach the spooler, then saves the preferences.
    pub fn save_state(&mut self) {
        print::finish_pending_jobs();
        save_prefs(&self.app, &self.launch);
    }

    /// False while a print job is in flight or a workbook has unsaved changes.
    pub fn can_close(&self) -> bool {
        !self.app.printing.busy() && !self.app.session.documents().iter().any(|d| d.is_dirty())
    }
}

fn load_prefs(app: &mut SheetApp, launch: &Launch) {
    if !launch.prefs {
        return;
    }
    // First run (or a ui.json without a usable language): follow the desktop's language.
    app.ui.language = iridium_ui_egui::i18n::Language::system();
    // ...and its light or dark appearance (#239), until a theme is chosen and saved in ui.json.
    app.ui.system_theme = true;
    let Some(dir) = launch.config_dir.as_deref() else { return };
    if let Ok(b) = std::fs::read(dir.join("ui.json"))
        && let Ok(ui) = serde_json::from_slice::<iridium_ui_egui::UiState>(&b)
    {
        let saved = serde_json::from_slice(&b).ok().and_then(|v| iridium_ui_egui::i18n::saved_language(&v));
        let language = saved.unwrap_or(app.ui.language);
        app.ui = ui.at_startup();
        app.ui.language = language;
    }
    if let Ok(b) = std::fs::read(dir.join("prefs.json"))
        && let Ok(p) = serde_json::from_slice(&b)
    {
        app.session.prefs = p;
    }
}

fn save_prefs(app: &SheetApp, launch: &Launch) {
    if !launch.prefs {
        return;
    }
    let Some(dir) = launch.config_dir.as_deref() else { return };
    let _ = std::fs::create_dir_all(dir);
    if let Ok(b) = serde_json::to_vec_pretty(&app.ui) {
        let _ = std::fs::write(dir.join("ui.json"), b);
    }
    if let Ok(b) = serde_json::to_vec_pretty(&app.session.prefs) {
        let _ = std::fs::write(dir.join("prefs.json"), b);
    }
}
/// Read the system clipboard through the shared handle, creating it on first use.
fn read_clipboard<T>(
    handle: &RefCell<Option<arboard::Clipboard>>,
    read: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Option<T> {
    let mut clipboard = handle.try_borrow_mut().ok()?;
    if clipboard.is_none() {
        *clipboard = arboard::Clipboard::new().ok();
    }
    read(clipboard.as_mut()?).ok()
}

fn services() -> Services {
    // Keep the owner alive: X11/Wayland serve clipboard data from this handle.
    let clipboard: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    let (html_clipboard, text_clipboard, image_clipboard) = (clipboard.clone(), clipboard.clone(), clipboard.clone());
    Services {
        copy_html: Some(Box::new(move |html, text| {
            let mut clipboard = clipboard.borrow_mut();
            if clipboard.is_none() {
                *clipboard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
            }
            clipboard.as_mut().ok_or("clipboard unavailable")?.set_html(html, Some(text)).map_err(|e| e.to_string())
        })),
        paste_html: Some(Box::new(move || read_clipboard(&html_clipboard, |clipboard| clipboard.get().html()))),
        paste_text: Some(Box::new(move || read_clipboard(&text_clipboard, |clipboard| clipboard.get_text()))),
        paste_image: Some(Box::new(move || {
            let image = read_clipboard(&image_clipboard, |clipboard| clipboard.get_image())?;
            image::RgbaImage::from_raw(u32::try_from(image.width).ok()?, u32::try_from(image.height).ok()?, image.bytes.into_owned())
        })),
        pick_open: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Spreadsheets", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt", "slk", "json"])
                .add_filter("Excel Workbook", &["xlsx", "xlsm"])
                .add_filter("Excel 97-2003 Workbook (import)", &["xls"])
                .add_filter("Excel Binary Workbook (data import)", &["xlsb"])
                .add_filter("OpenDocument Spreadsheet (data import)", &["ods"])
                .add_filter("CSV", &["csv"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_picture: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"])
                .pick_file()
                .and_then(|p| p.to_str().map(str::to_string))
        })),
        pick_save: Some(Box::new(|suggested: &str| {
            let path = std::path::Path::new(suggested);
            let stem = path.file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Book1").to_string();
            // The suggested name's type (an export asks for .pdf, .csv or .html) comes first.
            const TYPES: [(&str, &str); 4] =
                [("Excel Workbook (.xlsx)", "xlsx"), ("CSV UTF-8 (.csv)", "csv"), ("Web Page (.html)", "html"), ("PDF (.pdf)", "pdf")];
            let ext = path.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).filter(|e| TYPES.iter().any(|(_, x)| x == e));
            let ext = ext.unwrap_or_else(|| "xlsx".to_string());
            let mut dialog = rfd::FileDialog::new().set_file_name(format!("{stem}.{ext}"));
            for (label, x) in TYPES.iter().filter(|(_, x)| *x == ext).chain(TYPES.iter().filter(|(_, x)| *x != ext)) {
                dialog = dialog.add_filter(*label, &[*x]);
            }
            dialog.save_file().and_then(|p| p.to_str().map(str::to_string))
        })),
        open_url: Some(Box::new(|url: &str| open_with_system(url))),
        gsheets: vec![std::sync::Arc::new(gsheets::DesktopBackend::google()), std::sync::Arc::new(gsheets::DesktopBackend::excel())],
        connectors: vec![std::sync::Arc::new(connector::DesktopBackend::new())],
        printers: Some(print::discover()),
        print_native: Some(print::submit()),
        ..Default::default()
    }
}

/// Opens a URL or file with the system's default application.
fn open_with_system(target: &str) {
    // Last line of defence behind `link.open` and the click prompt: the system only ever gets
    // http, https and mailto addresses.
    let iridium_engine::links::LinkKind::External(target) = iridium_engine::links::classify(target) else {
        log::warn!("refused to open {target:?}: only http, https and mailto links are opened");
        return;
    };
    let target = target.as_str();
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(cmd).arg(target).spawn() {
        log::warn!("{cmd} {target}: {e}");
    }
}
