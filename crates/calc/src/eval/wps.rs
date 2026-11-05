//! The WPS functions that need the workbook: SHEETSNAME and BOOKNAME.
//!
//! WPS added both in 2024 (its "WPS独有函数" releases). They are here
//! rather than in the function crate because they read the workbook, which
//! the function crate cannot see.
//!
//! - <https://bbs.wps.cn/topic/33990> — "WPS又新增两个独有函数"

use iridium_core::{Array, CellError, Value};

use super::{Ev, Evaluator, Expr, err};

/// SHEETSNAME([reference], [n]): the name of a sheet.
///
/// With no reference the names of every sheet in the workbook, in order, as
/// an array. With `n` the nth of them, counting from 1. A reference names
/// the sheet it points at, and `n` then counts within that.
pub(super) fn sheetsname(ev: &mut Evaluator, args: &[Expr]) -> Ev {
    // The nth name, when the second argument asks for one.
    let which = match args.get(1) {
        None | Some(Expr::Missing) => None,
        Some(_) => match ev.arg_val(args, 1) {
            Some(v) => v.scalar().to_number().ok(),
            None => None,
        },
    };

    // A reference names the sheet it points at.
    let named = match args.first() {
        None | Some(Expr::Missing) => None,
        Some(expr) => match ev.eval(expr) {
            Ev::R(areas) => areas.first().copied().map(|area| area.sheet),
            _ => None,
        },
    };

    let wb = ev.host.workbook();
    let names: Vec<Value> = wb.sheets.iter().map(|s| Value::text(s.name.clone())).collect();

    match which {
        Some(n) => {
            let n = n.trunc();
            // With a reference the count picks that sheet's own name;
            // without one, the workbook's sheets in order.
            let list: Vec<Value> = match named {
                Some(sheet) => wb.sheet(sheet).map(|s| vec![Value::text(s.name.clone())]).unwrap_or_default(),
                None => names.clone(),
            };
            if n < 1.0 || n > list.len() as f64 {
                return err(CellError::Value);
            }
            Ev::V(list[(n as usize) - 1].clone())
        }
        None => {
            // Every sheet's name, as an array. A workbook with one sheet
            // gives that one name, not a one-element array: WPS returns the
            // name itself when there is only one sheet.
            if names.len() == 1 {
                Ev::V(names[0].clone())
            } else {
                match Array::new(names.len(), 1, names) {
                    Some(a) => Ev::V(Value::from(a)),
                    None => err(CellError::Calc),
                }
            }
        }
    }
}

/// BOOKNAME(): the workbook's name.
///
/// The file's name without its path or extension, which is what WPS
/// returns: the long formula `MID(CELL("filename",A1),FIND("[",...)+1,...)`
/// is what BOOKNAME replaces.
pub(super) fn bookname(ev: &mut Evaluator, _args: &[Expr]) -> Ev {
    let wb = ev.host.workbook();
    let Some(file) = &wb.source_path else {
        // No file yet: Excel reports an empty string until the workbook has
        // been saved once, and so does this.
        return Ev::V(Value::text(""));
    };
    let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    // The extension is not part of the name.
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if !ext.is_empty() && !stem.is_empty() => stem.to_string(),
        _ => name,
    };
    Ev::V(Value::text(stem))
}

/// EVALUATE(formula_text): the value of the formula the text holds.
///
/// Excel's own function, which WPS also has. The text is parsed and
/// evaluated where the formula stands, so it sees that cell's own names and
/// references.
pub(super) fn evaluate(ev: &mut Evaluator, args: &[Expr]) -> Ev {
    let text = match ev.arg_val(args, 0).map(|v| v.scalar().to_text()) {
        Some(Ok(t)) => t,
        Some(Err(e)) => return err(e),
        None => return err(CellError::Value),
    };
    let body = text.strip_prefix('=').unwrap_or(&text);
    match iridium_formula::parse(body) {
        Ok(expr) => {
            let v = ev.eval(&expr);
            match v {
                Ev::V(v) => Ev::V(v),
                // A reference evaluates to what it points at.
                Ev::R(areas) => {
                    let Some(area) = areas.first().copied() else { return err(CellError::Ref) };
                    Ev::V(ev.host.cell_value(area.sheet, area.range.start))
                }
                Ev::L(_) => err(CellError::Value),
            }
        }
        Err(_) => err(CellError::Name),
    }
}
