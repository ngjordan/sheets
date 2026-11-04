//! GETPIVOTDATA: reads a value out of a PivotTable report.
//!
//! A pivot's report is written onto the sheet as ordinary cells: a header row
//! carrying the value fields' captions ("Sum of Sales"), the leftmost column
//! carrying the row labels, and the body carrying the numbers. Excel's
//! GETPIVOTDATA looks a value up there:
//!
//! ```text
//! GETPIVOTDATA("Sum of Sales", "North")        // the value for the "North" row
//! GETPIVOTDATA("Sum of Sales", "North", "Q1")  // and the "Q1" column
//! ```
//!
//! The first argument names the PivotTable, or points at a cell inside its
//! report; the second is the value field's caption; the rest are the row and
//! column items. With no item the grand total is returned.
//!
//! It is implemented here rather than in the function crate because it needs
//! the workbook: the pivot's report and the cells it wrote.

use gridcraft_core::{CellError, CellRef, RangeRef, Value};
use gridcraft_model::Workbook;

use super::{Ev, Evaluator, Expr, err};

/// GETPIVOTDATA(data_field, [item1], [item2], …).
pub(super) fn getpivotdata(ev: &mut Evaluator, args: &[Expr]) -> Ev {
    // The value field's caption.
    let Some(Ok(field)) = ev.arg_val(args, 0).map(|v| v.scalar().to_text()) else {
        return err(CellError::Value);
    };
    let field = field.trim().to_string();
    if field.is_empty() {
        return err(CellError::Value);
    }

    // The pivot: either a reference to a cell inside its report, or its name.
    let (name, at) = match args.get(1).map(|a| ev.eval(a)) {
        Some(Ev::R(areas)) => {
            let Some(area) = areas.first().copied() else { return err(CellError::Ref) };
            (String::new(), Some((area.sheet, area.range.start)))
        }
        _ => {
            let Some(Ok(text)) = ev.arg_val(args, 1).map(|v| v.scalar().to_text()) else {
                return err(CellError::Value);
            };
            (text.trim().to_string(), None)
        }
    };

    // The items: the row label, then the column label.
    let mut items = Vec::new();
    for i in 2..args.len() {
        let Some(Ok(text)) = ev.arg_val(args, i).map(|v| v.scalar().to_text()) else {
            return err(CellError::Value);
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        items.push(text);
    }

    let wb = ev.host.workbook();
    let Some((sheet, idx)) = find_pivot(wb, &name, at) else { return err(CellError::Ref) };
    let Some(pt) = wb.sheet(sheet).and_then(|s| s.pivots.get(idx)) else { return err(CellError::Ref) };
    let Some(area) = pt.last_range else { return err(CellError::Ref) };

    match lookup(wb, sheet, area, &field, &items) {
        Some(v) => Ev::V(v),
        None => err(CellError::Ref),
    }
}

/// The PivotTable named `name`, or the one whose report holds the cell `at`.
///
/// `at` is `(sheet, cell)` of a reference argument; `None` when the argument
/// was a name.
fn find_pivot(wb: &Workbook, name: &str, at: Option<(usize, CellRef)>) -> Option<(usize, usize)> {
    let mut by_cell = None;
    for (si, sheet) in wb.sheets.iter().enumerate() {
        for (pi, pt) in sheet.pivots.iter().enumerate() {
            if !name.is_empty() && pt.name.eq_ignore_ascii_case(name) {
                return Some((si, pi));
            }
            if by_cell.is_none()
                && let Some((cs, c)) = at
                && cs == si
                && pt.last_range.is_some_and(|r| r.contains(c))
            {
                by_cell = Some((si, pi));
            }
        }
    }
    by_cell
}

/// The cell's text, or an empty string.
fn text(wb: &Workbook, sheet: usize, at: CellRef) -> String {
    wb.sheet(sheet).and_then(|s| s.cell(at)).map(|c| c.value.to_text().unwrap_or_default()).unwrap_or_default()
}

/// The cell's value, or `Value::Empty`.
fn value(wb: &Workbook, sheet: usize, at: CellRef) -> Value {
    wb.sheet(sheet).and_then(|s| s.cell(at)).map(|c| c.value.clone()).unwrap_or(Value::Empty)
}

/// Looks `field` and `items` up in the pivot's report area.
///
/// A pivot's report has two shapes. With no column field the value field's
/// caption sits above the values, in the value column. With column fields
/// the caption sits in the corner above the row labels, the column labels
/// are on the next row, and the values sit under them. Both are read here.
fn lookup(wb: &Workbook, sheet: usize, area: RangeRef, field: &str, items: &[String]) -> Option<Value> {
    let label_col = area.start.col;

    // The value field's caption names the field. It is the first cell in the
    // report that carries it.
    let mut caption = None;
    for r in area.start.row..=area.end.row {
        for c in area.start.col..=area.end.col {
            if text(wb, sheet, CellRef::new(r, c)).eq_ignore_ascii_case(field) {
                caption = Some((r, c));
                break;
            }
        }
        if caption.is_some() {
            break;
        }
    }
    let (caption_row, caption_col) = caption?;

    // The column labels, when the report has column fields, are on the row
    // below the caption.
    let header_row = caption_row + 1;
    let column_label = |c: u32| -> String { text(wb, sheet, CellRef::new(header_row, c)).trim().to_string() };

    // With no item: the grand total. It is the row labelled "Grand Total";
    // its value sits under the "Grand Total" column when the report has
    // column fields, and under the caption when it does not.
    if items.is_empty() {
        let mut total_row = None;
        for r in caption_row..=area.end.row {
            if text(wb, sheet, CellRef::new(r, label_col)).to_ascii_lowercase().contains("grand total") {
                total_row = Some(r);
                break;
            }
        }
        let total_row = total_row?;
        let mut total_col = caption_col;
        for c in area.start.col..=area.end.col {
            if column_label(c).eq_ignore_ascii_case("grand total") {
                total_col = c;
                break;
            }
        }
        return Some(value(wb, sheet, CellRef::new(total_row, total_col)));
    }

    // The row label is in the report's first column. It may sit on the
    // caption's row or below it.
    let mut row = None;
    for r in caption_row..=area.end.row {
        if text(wb, sheet, CellRef::new(r, label_col)).trim().eq_ignore_ascii_case(items[0].trim()) {
            row = Some(r);
            break;
        }
    }
    let row = row?;

    // With a second item it names the column: the column whose header (on
    // the row below the caption) matches.
    let target_col = if items.len() > 1 {
        let mut found = None;
        for c in area.start.col..=area.end.col {
            if column_label(c).eq_ignore_ascii_case(items[1].trim()) {
                found = Some(c);
                break;
            }
        }
        found?
    } else {
        caption_col
    };

    Some(value(wb, sheet, CellRef::new(row, target_col)))
}
