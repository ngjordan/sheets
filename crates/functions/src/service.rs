//! The Excel functions that need something Iridium Sheets does not have.
//!
//! `TRANSLATE`, `IMAGE` and `STOCKHISTORY` are service-backed in Excel too
//! (Microsoft Translator, a URL fetch, a market-data provider), and `RTD`,
//! `CALL` and `REGISTER.ID` need a COM server or a DLL the app does not load.
//!
//! Each is registered with its Excel signature so a workbook that uses one
//! loads and shows the same error Excel would show when its service is
//! missing, rather than failing to parse. `DETECTLANGUAGE` is here too
//! because Excel backs it with the same service, but it works offline: it
//! reads the script and the function words instead of calling out.

use gridcraft_core::{CellError, Value};

use crate::VAR;
use crate::util::{R, S, arg, text};

/// DETECTLANGUAGE(text): the language of a text, as an Excel language code.
fn detectlanguage(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let s = text(a, 0)?;
    match super::language::detect(&s) {
        Some(code) => Ok(Value::from(code)),
        None => Err(CellError::Value),
    }
}

/// TRANSLATE(text, [source_language], [target_language]).
///
/// Needs a translation service, which the app does not have.
fn translate(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = text(a, 0)?;
    Err(CellError::Value)
}

/// IMAGE(source, [alt_text], [sizing], [height], [width]).
///
/// Needs a URL fetch and a picture in the cell, which the app does not have.
fn image(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = text(a, 0)?;
    Err(CellError::Value)
}

/// STOCKHISTORY(stock, start_date, [end_date], [interval], [headers], [properties], ...).
///
/// Needs a market-data provider, which the app does not have.
fn stockhistory(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = text(a, 0)?;
    Err(CellError::Value)
}

/// RTD(ProgID, server, topic1, [topic2], ...).
///
/// Needs a COM automation server, which only Windows has.
fn rtd(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = text(a, 0)?;
    Err(CellError::Value)
}

/// CALL(name, [argument1], ...).
///
/// Needs a registered DLL function, which the app does not load.
fn call(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = arg(a, 0)?;
    Err(CellError::Value)
}

/// REGISTER.ID(name, [type1], [type2], ...).
///
/// Needs a registered DLL function, which the app does not load.
fn register_id(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = arg(a, 0)?;
    Err(CellError::Value)
}

pub(crate) fn specs() -> Vec<crate::FnSpec> {
    vec![
        f!(
            "DETECTLANGUAGE",
            1,
            1,
            Text,
            S,
            "DETECTLANGUAGE(text)",
            "The language of a text, as a language code (read offline; not the Translator service).",
            detectlanguage
        ),
        f!(
            "TRANSLATE",
            1,
            3,
            Text,
            S,
            "TRANSLATE(text, [source_language], [target_language])",
            "Translates text into another language (no translation service; returns #VALUE!).",
            translate
        ),
        f!(
            "IMAGE",
            1,
            5,
            Web,
            S,
            "IMAGE(source, [alt_text], [sizing], [height], [width])",
            "Inserts a picture from a URL into a cell (no URL fetch; returns #VALUE!).",
            image
        ),
        f!(
            "STOCKHISTORY",
            2,
            VAR,
            Financial,
            S,
            "STOCKHISTORY(stock, start_date, [end_date], [interval], [headers], [properties], ...)",
            "Historical stock prices for an instrument (no market-data provider; returns #VALUE!).",
            stockhistory
        ),
        f!(
            "RTD",
            3,
            VAR,
            Web,
            S,
            "RTD(ProgID, server, topic1, [topic2], ...)",
            "Real-time data from a COM automation server (no COM server; returns #VALUE!).",
            rtd
        ),
        f!("CALL", 1, VAR, AddIn, S, "CALL(name, [argument1], ...)", "Calls a registered DLL function (no DLL runtime; returns #VALUE!).", call),
        f!(
            "REGISTER.ID",
            1,
            VAR,
            AddIn,
            S,
            "REGISTER.ID(name, [type1], [type2], ...)",
            "The register id of a registered DLL function (no DLL runtime; returns #VALUE!).",
            register_id
        ),
    ]
}

#[cfg(test)]
mod tests {
    use crate::util::testutil::*;
    use gridcraft_core::{CellError, Value};

    #[test]
    fn detectlanguage_names_the_language() {
        assert_eq!(ev("DETECTLANGUAGE", vec![t("Hola mundo! ¿Cómo estás?")]), Value::text("es"));
        assert_eq!(ev("DETECTLANGUAGE", vec![t("Это текст на русском языке")]), Value::text("ru"));
        assert_eq!(ev("DETECTLANGUAGE", vec![t("")]), Value::Error(CellError::Value));
    }

    #[test]
    fn the_service_backed_ones_give_the_error_excel_gives_without_the_service() {
        assert_eq!(ev("TRANSLATE", vec![t("Hello")]), Value::Error(CellError::Value));
        assert_eq!(ev("IMAGE", vec![t("https://example.com/a.png")]), Value::Error(CellError::Value));
        assert_eq!(ev("STOCKHISTORY", vec![t("MSFT"), n(45000.0)]), Value::Error(CellError::Value));
        assert_eq!(ev("RTD", vec![t("Prog.ID"), t("server"), t("topic")]), Value::Error(CellError::Value));
        assert_eq!(ev("CALL", vec![t("kernel32.dll"), t("Sleep")]), Value::Error(CellError::Value));
        assert_eq!(ev("REGISTER.ID", vec![t("kernel32.dll"), t("Sleep")]), Value::Error(CellError::Value));
    }
}
