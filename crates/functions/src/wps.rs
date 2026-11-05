//! The functions WPS Office Spreadsheet has and Microsoft Excel does not.
//!
//! These come from WPS's own function reference and its official community
//! posts ("WPS独有函数"). They are here so a workbook written in WPS opens
//! and calculates in Iridium Sheets.
//!
//! The ones that need something Iridium Sheets does not have (a translation
//! service, a market-data feed, an AI backend) are registered with their WPS
//! signature and give the error WPS gives when its service is missing. The
//! rest work offline.
//!
//! Sources:
//! - <https://bbs.wps.cn/topic/28346> — "WPS独有函数，非常实用！" (the
//!   exclusive functions, with worked examples)
//! - <https://bbs.wps.cn/topic/33990> — "WPS又新增两个独有函数" (SHEETSNAME,
//!   BOOKNAME)
//! - <https://www.wps.ai/en-us/docs/wps-sheets/formulas-and-functions/>

use gridcraft_core::{CellError, Value};

use crate::VAR;
use crate::text::TJ;
use crate::util::{A, MAX_CELLS, R, S, arg, num, text};

/// ROUNDBANK(number, num_digits): round by the banker's rule.
///
/// Also called "round half to even" (四舍六入五成双): a value exactly half
/// way rounds to the even neighbour, so repeated rounding does not drift
/// upwards. Excel's ROUND always rounds a half away from zero, so this is
/// the one rounding rule Excel cannot do.
fn roundbank(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let x = num(a, 0)?;
    let digits = num(a, 1)?;
    if !x.is_finite() {
        return Err(CellError::Num);
    }
    Ok(Value::number(round_half_even(x, digits)))
}

/// Rounds `x` to `digits` decimals, a half going to the even neighbour.
///
/// The scaling is done on the decimal string, not on the float: `2.675` is
/// really `2.67499999999999982236431605997495353221893310546875`, and
/// multiplying by 100 and rounding would give 267.5 -> 268, where the
/// decimal reading gives 267.5 -> 268 as well, but `1.005` differs. Excel
/// rounds the decimal reading, so this does too.
fn round_half_even(x: f64, digits: f64) -> f64 {
    let d = digits.trunc();
    if !d.is_finite() {
        return x;
    }
    let d = d.clamp(-308.0, 308.0) as i32;
    // A value too large to scale keeps its own digits; the multiplication
    // would overflow.
    if d >= 308 {
        return x;
    }
    let factor = 10f64.powi(d);
    let scaled = x * factor;
    if !scaled.is_finite() {
        return x;
    }
    // The decimal reading of the scaled value, to the places that matter.
    let rounder = 15 - d.clamp(0, 15);
    let rounded = format!("{scaled:.*}", rounder as usize).parse::<f64>().unwrap_or(scaled);
    let floor = rounded.floor();
    let frac = rounded - floor;
    // A half goes to the even neighbour; anything else to the nearer one.
    let result = if frac > 0.5 {
        floor + 1.0
    } else if frac < 0.5 || (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    };
    result / factor
}

/// SUBSTITUTES(text, old_texts, [new_texts]): replace several parts at once.
///
/// Each part of `old_texts` is replaced by the part of `new_texts` at the
/// same place; a new text left out leaves that part unchanged. With one
/// array of olds and one of news it replaces pairwise; with one array of
/// olds and one new text it replaces every old with that text.
fn substitutes(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let subject = text(a, 0)?;
    let olds = strings(a, 1)?;
    let news = strings(a, 2)?;
    let mut out = subject;
    for (i, old) in olds.iter().enumerate() {
        if old.is_empty() {
            continue;
        }
        // The news pair with the olds one for one. When there is one news
        // and several olds, it replaces them all.
        let new = if news.len() == 1 && olds.len() > 1 { news[0].clone() } else { news.get(i).cloned().unwrap_or_default() };
        out = out.replace(old.as_str(), &new);
    }
    Ok(Value::from(out))
}

/// The text of each cell of an argument, in order.
///
/// WPS passes the olds and the news as arrays (`{"a","b","c"}`), so an
/// argument that holds one value is read as a one-element array.
fn strings(a: &[crate::Arg], i: usize) -> R<Vec<String>> {
    let v = &arg(a, i)?.value;
    if let Value::Error(e) = v {
        return Err(*e);
    }
    crate::util::as_array(v).iter().map(|v| v.to_text()).collect()
}

/// REPTARRAY(array, count): the array, repeated `count` times.
///
/// The array is stacked: `REPTARRAY(SEQUENCE(3), 4)` gives twelve values,
/// the three repeated four times over.
fn reptarray(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let first = arg(a, 0)?;
    let count = num(a, 1)?;
    if !count.is_finite() || count < 0.0 {
        return Err(CellError::Value);
    }
    let count = count.trunc() as usize;
    let Value::Array(arr) = &first.value else {
        // A single value repeated is just that value.
        if count == 0 {
            return Ok(Value::Empty);
        }
        return Ok(first.value.clone());
    };
    if count == 0 {
        return Ok(Value::Empty);
    }
    // The result cannot be bigger than a sheet. Without the check,
    // REPTARRAY(SEQUENCE(3), 1E18) asks for a vector of three quintillion
    // values and the allocation aborts the process.
    let total = arr.data.len().saturating_mul(count);
    if total > MAX_CELLS {
        return Err(CellError::Num);
    }
    let mut data = Vec::with_capacity(total);
    for _ in 0..count {
        data.extend(arr.data.iter().cloned());
    }
    let out = gridcraft_core::Array::new(arr.rows.saturating_mul(count), arr.cols, data).ok_or(CellError::Calc)?;
    Ok(Value::from(out))
}

/// RMB(number): an amount in Chinese capital numerals.
///
/// The form used on Chinese receipts and payment slips: 壹佰贰拾叁元肆角伍分.
/// The `type` argument picks the form: 0 for the capital numerals, 1 for the
/// everyday ones, and 2 for the financial ones with the 角 and 分 places.
fn rmb(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let x = num(a, 0)?;
    let kind = if a.len() > 1 { num(a, 1)? } else { 0.0 };
    if !x.is_finite() {
        return Err(CellError::Num);
    }
    Ok(Value::from(capital_rmb(x, kind.trunc() as i32)))
}

/// The capital and everyday numerals.
const DIGITS_CAPITAL: [char; 10] = ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'];
const DIGITS_EVERYDAY: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const DIGITS_FINANCIAL: [char; 10] = ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'];
const UNITS: [&str; 8] = ["", "拾", "佰", "仟", "万", "亿", "万亿", "亿亿"];
const UNITS_FINANCIAL: [&str; 8] = ["", "拾", "佰", "仟", "万", "亿", "万亿", "亿亿"];

/// An amount in Chinese numerals, as a Chinese receipt writes it.
fn capital_rmb(x: f64, kind: i32) -> String {
    let (digits, units) = match kind {
        1 => (DIGITS_EVERYDAY, &UNITS),
        2 => (DIGITS_FINANCIAL, &UNITS_FINANCIAL),
        _ => (DIGITS_CAPITAL, &UNITS),
    };
    let negative = x < 0.0;
    // The amount is in yuan, with two decimal places.
    let cents = (x.abs() * 100.0).round() as u64;
    let (yuan, jiao, fen) = (cents / 100, (cents % 100) / 10, cents % 10);

    let mut out = String::new();
    if negative {
        out.push('负');
    }
    out.push_str(&integer_chinese(yuan, &digits, units));
    out.push('元');
    if jiao == 0 && fen == 0 {
        out.push('整');
        return out;
    }
    if jiao > 0 {
        out.push(digits[jiao as usize]);
        out.push('角');
    } else if fen > 0 {
        // Nothing in the 角 place, but there is a 分: a zero goes in first.
        out.push('零');
    }
    if fen > 0 {
        out.push(digits[fen as usize]);
        out.push('分');
    }
    out
}

/// A whole number in Chinese numerals.
fn integer_chinese(n: u64, digits: &[char; 10], units: &[&str; 8]) -> String {
    if n == 0 {
        return digits[0].to_string();
    }
    // Split into groups of four digits, lowest first.
    let mut groups = Vec::new();
    let mut rest = n;
    while rest > 0 {
        groups.push((rest % 10_000) as u32);
        rest /= 10_000;
    }
    let mut out = String::new();
    for gi in (0..groups.len()).rev() {
        let g = groups[gi];
        if g == 0 {
            // A zero group in the middle still needs its zero, unless nothing
            // has been written yet or everything after it is zero too.
            if !out.is_empty() && groups[..gi].iter().any(|x| *x != 0) {
                out.push(digits[0]);
            }
            continue;
        }
        let mut part = String::new();
        let mut zero_pending = false;
        for (i, d) in (0..4).rev().map(|i| (i, (g / 10u32.pow(i)) % 10)).rev() {
            let _ = i;
            let _ = d;
        }
        // Write the four digits of the group, highest first.
        let mut started = false;
        for i in (0..4).rev() {
            let d = (g / 10u32.pow(i)) % 10;
            if d == 0 {
                if started {
                    zero_pending = true;
                }
                continue;
            }
            if zero_pending {
                part.push(digits[0]);
                zero_pending = false;
            }
            part.push(digits[d as usize]);
            // The 拾/佰/仟 place marker. The 10^0 digit takes none.
            if i > 0 {
                part.push_str(units[i as usize]);
            }
            started = true;
        }
        if !part.is_empty() {
            if !out.is_empty() {
                out.push(digits[0]);
            }
            out.push_str(&part);
            // The group's own unit: 万 for the second group, 亿 for the
            // third, and nothing for the first.
            if gi > 0 {
                out.push_str(units[gi + 3]);
            }
        }
    }
    out
}

/// REGEXP(text, pattern, [replacement]): a regular expression.
///
/// WPS's own take on Excel's REGEXEXTRACT/REGEXREPLACE/REGEXTEST, taking
/// the pattern as its second argument and applying it to the whole text.
fn regexp(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let subject = text(a, 0)?;
    let pattern = text(a, 1)?;
    let replacement = if a.len() > 2 { text(a, 2)? } else { String::new() };
    let re = regex::Regex::new(&pattern).map_err(|_| CellError::Value)?;
    if replacement.is_empty() {
        // Without a replacement it extracts: the first match, or the whole
        // text when the pattern matches it.
        return Ok(Value::from(match re.find(&subject) {
            Some(m) => m.as_str().to_string(),
            None => String::new(),
        }));
    }
    Ok(Value::from(re.replace_all(&subject, replacement.as_str()).into_owned()))
}

/// WPSAI.CLASSIFY(cell, categories): names the category a value falls in.
///
/// WPS's own service-backed AI functions: they need WPS's AI backend, which
/// Iridium Sheets does not have, so they give the error WPS gives when its
/// service is unreachable.
fn wpsai(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = arg(a, 0)?;
    Err(CellError::Value)
}

/// FIELDVALUE(field, [source]): a field of a linked stock or fund record.
///
/// Needs the market-data service behind WPS's linked records.
fn fieldvalue(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let _ = arg(a, 0)?;
    Err(CellError::Value)
}

/// Calls the function `base` under another name.
///
/// WPS gives some of Excel's functions a name of its own (MROUD for MROUND,
/// SERCH for SEARCH, and so on). The alias is the base function, so the
/// behaviour is Excel's exactly.
fn alias(base: &'static str, a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    let spec = crate::lookup(base).ok_or(CellError::Value)?;
    Ok(crate::call(spec, a, c))
}

/// MROUD(number, multiple): WPS's name for MROUND.
fn mround_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("MROUND", a, c)
}

/// SERCH(find_text, within_text, [start_num]): WPS's name for SEARCH.
fn search_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("SEARCH", a, c)
}

/// SERCHB(find_text, within_text, [start_num]): WPS's name for SEARCHB.
fn searchb_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("SEARCHB", a, c)
}

/// STEDV(number1, [number2], ...): WPS's name for STDEV.
fn stdev_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("STDEV", a, c)
}

/// PRODUT(number1, [number2], ...): WPS's name for PRODUCT.
fn product_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("PRODUCT", a, c)
}

/// TEXJOIN(delimiter, ignore_empty, text1, ...): WPS's name for TEXTJOIN.
fn textjoin_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("TEXTJOIN", a, c)
}

/// DDD(start_date, end_date, [method]): WPS's name for DAYS360.
fn days360_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("DAYS360", a, c)
}

/// DEVDQ(number1, [number2], ...): WPS's name for DEVSQ.
fn devsq_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("DEVSQ", a, c)
}

/// ODDFRRICE(...): WPS's name for ODDFPRICE.
fn oddfrprice_alias(a: &[crate::Arg], c: &mut dyn crate::Ctx) -> R<Value> {
    alias("ODDFPRICE", a, c)
}

/// WIDECHAR(text): half-width characters as full-width ones.
///
/// WPS's own function (WPS's name for Excel's WIDECHAR, which Excel has
/// only in its double-byte builds). An ASCII letter or digit becomes its
/// full-width form, and half-width katakana becomes full-width katakana;
/// everything else is left alone.
fn widechar(a: &[crate::Arg], _c: &mut dyn crate::Ctx) -> R<Value> {
    let s = text(a, 0)?;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            // ASCII: 0x21..=0x7E becomes 0xFF01..=0xFF5E.
            '!'..='~' => out.push(char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch)),
            // The half-width katakana: 0xFF61..=0xFF9F becomes 0x30A1..=0x30DF.
            '\u{FF61}'..='\u{FF9F}' => out.push(char::from_u32(ch as u32 - 0xFF61 + 0x30A1).unwrap_or(ch)),
            // The half-width space.
            '\u{FF20}' => out.push('\u{3000}'),
            _ => out.push(ch),
        }
    }
    Ok(Value::from(out))
}

pub(crate) fn specs() -> Vec<crate::FnSpec> {
    vec![
        f!(
            "ROUNDBANK",
            1,
            2,
            MathTrig,
            S,
            "ROUNDBANK(number, num_digits)",
            "Rounds a number by the banker's rule (a half goes to the even neighbour).",
            roundbank
        ),
        f!(
            "SUBSTITUTES",
            3,
            3,
            Text,
            A,
            "SUBSTITUTES(text, old_texts, [new_texts])",
            "Replaces several parts of a text at once, each with its own new text.",
            substitutes
        ),
        f!("REPTARRAY", 2, 2, Text, A, "REPTARRAY(array, count)", "An array, repeated the given number of times.", reptarray),
        f!("RMB", 1, 2, Text, S, "RMB(number, [type])", "An amount in Chinese capital numerals, as a Chinese receipt writes it.", rmb),
        f!(
            "REGEXP",
            2,
            3,
            Text,
            S,
            "REGEXP(text, pattern, [replacement])",
            "Matches a regular expression (extracts the first match, or replaces when a replacement is given).",
            regexp
        ),
        f!(
            "WPSAI.CLASSIFY",
            2,
            2,
            Text,
            S,
            "WPSAI.CLASSIFY(cell, categories)",
            "Names the category a value falls in (no AI service; returns #VALUE!).",
            wpsai
        ),
        f!(
            "WPSAI.EXTRACT",
            1,
            2,
            Text,
            S,
            "WPSAI.EXTRACT(cell, [instruction])",
            "Extracts structured fields from a cell's text (no AI service; returns #VALUE!).",
            wpsai
        ),
        f!(
            "WPSAI.SUMMARIZE",
            1,
            2,
            Text,
            S,
            "WPSAI.SUMMARIZE(cell, [instruction])",
            "Summarises a cell's text (no AI service; returns #VALUE!).",
            wpsai
        ),
        f!("MROUD", 2, 2, MathTrig, S, "MROUD(number, multiple)", "Rounds a number to the nearest multiple (WPS's name for MROUND).", mround_alias),
        f!(
            "SERCH",
            2,
            3,
            Text,
            S,
            "SERCH(find_text, within_text, [start_num])",
            "The position of text in another text (WPS's name for SEARCH).",
            search_alias
        ),
        f!(
            "SERCHB",
            2,
            3,
            Text,
            S,
            "SERCHB(find_text, within_text, [start_num])",
            "The position of text in another text, by byte (WPS's name for SEARCHB).",
            searchb_alias
        ),
        f!(
            "STEDV",
            1,
            VAR,
            Compatibility,
            A,
            "STEDV(number1, [number2], ...)",
            "The sample standard deviation of the numbers (WPS's name for STDEV).",
            stdev_alias
        ),
        f!("PRODUT", 1, VAR, MathTrig, A, "PRODUT(number1, [number2], ...)", "The product of the numbers (WPS's name for PRODUCT).", product_alias),
        f!(
            "TEXJOIN",
            3,
            VAR,
            Text,
            TJ,
            "TEXJOIN(delimiter, ignore_empty, text1, ...)",
            "Joins text with a delimiter (WPS's name for TEXTJOIN).",
            textjoin_alias
        ),
        f!(
            "DDD",
            2,
            3,
            DateTime,
            S,
            "DDD(start_date, end_date, [method])",
            "The days between two dates, on a 360-day year (WPS's name for DAYS360).",
            days360_alias
        ),
        f!(
            "DEVDQ",
            1,
            VAR,
            Statistical,
            A,
            "DEVDQ(number1, [number2], ...)",
            "The sum of the squares of the deviations (WPS's name for DEVSQ).",
            devsq_alias
        ),
        f!(
            "ODDFRRICE",
            8,
            9,
            Financial,
            S,
            "ODDFRRICE(settlement, maturity, issue, first_coupon, rate, yld, redemption, frequency, [basis])",
            "The price per $100 of a security with an odd first coupon (WPS's name for ODDFPRICE).",
            oddfrprice_alias
        ),
        f!("WIDECHAR", 1, 1, Text, S, "WIDECHAR(text)", "The text with its half-width characters as full-width ones.", widechar),
        f!(
            "WPSAI.SENTIANALSIS",
            1,
            2,
            Text,
            S,
            "WPSAI.SENTIANALSIS(cell, [instruction])",
            "Reads the sentiment of a cell's text (no AI service; returns #VALUE!).",
            wpsai
        ),
        f!(
            "FIELDVALUE",
            1,
            2,
            Information,
            S,
            "FIELDVALUE(field, [source])",
            "A field of a linked stock or fund record (no market-data service; returns #VALUE!).",
            fieldvalue
        ),
    ]
}

#[cfg(test)]
mod tests {
    use crate::util::testutil::*;
    use gridcraft_core::{CellError, Value};

    #[test]
    fn roundbank_rounds_a_half_to_even() {
        // 2.5 -> 2 and 3.5 -> 4: the even neighbour.
        assert_eq!(ev("ROUNDBANK", vec![n(2.5), n(0.0)]), Value::Number(2.0));
        assert_eq!(ev("ROUNDBANK", vec![n(3.5), n(0.0)]), Value::Number(4.0));
        assert_eq!(ev("ROUNDBANK", vec![n(-2.5), n(0.0)]), Value::Number(-2.0));
        assert_eq!(ev("ROUNDBANK", vec![n(-3.5), n(0.0)]), Value::Number(-4.0));
        // An ordinary half away from zero, as ROUND does.
        assert_eq!(ev("ROUNDBANK", vec![n(2.4), n(0.0)]), Value::Number(2.0));
        assert_eq!(ev("ROUNDBANK", vec![n(2.6), n(0.0)]), Value::Number(3.0));
        // To two decimals.
        assert_eq!(ev("ROUNDBANK", vec![n(1.125), n(2.0)]), Value::Number(1.12));
        assert_eq!(ev("ROUNDBANK", vec![n(1.135), n(2.0)]), Value::Number(1.14));
    }

    #[test]
    fn substitutes_replaces_each_part() {
        // WPS passes the olds and the news as arrays.
        let olds = arr(vec![vec![tv("a")], vec![tv("b")], vec![tv("c")]]);
        let news = arr(vec![vec![tv("1")], vec![tv("2")], vec![tv("3")]]);
        assert_eq!(ev("SUBSTITUTES", vec![t("a-b-c"), av(olds), av(news)]), Value::text("1-2-3"));
        // One new text for every old text.
        let olds2 = arr(vec![vec![tv("a")], vec![tv("b")], vec![tv("c")]]);
        assert_eq!(ev("SUBSTITUTES", vec![t("a-b-c"), av(olds2), t("*")]), Value::text("*-*-*"));
    }

    #[test]
    fn reptarray_repeats_the_array() {
        // REPTARRAY(SEQUENCE(3), 4): three rows, four times over.
        let three = arr(vec![vec![nv(1.0)], vec![nv(2.0)], vec![nv(3.0)]]);
        let got = ev("REPTARRAY", vec![av(three), n(4.0)]);
        let Value::Array(a) = got else { panic!("expected an array") };
        assert_eq!(a.rows, 12);
        assert_eq!(a.cols, 1);
        assert_eq!(a.data[0], Value::Number(1.0));
        assert_eq!(a.data[3], Value::Number(1.0));
        assert_eq!(a.data[11], Value::Number(3.0));
    }

    #[test]
    fn rmb_writes_the_amount_in_capital_numerals() {
        assert_eq!(ev("RMB", vec![n(123.45)]), Value::text("壹佰贰拾叁元肆角伍分"));
        assert_eq!(ev("RMB", vec![n(0.0)]), Value::text("零元整"));
        assert_eq!(ev("RMB", vec![n(10.0)]), Value::text("壹拾元整"));
    }

    #[test]
    fn regexp_extracts_and_replaces() {
        assert_eq!(ev("REGEXP", vec![t("abc123"), t(r"\d+")]), Value::text("123"));
        assert_eq!(ev("REGEXP", vec![t("abc123"), t(r"\d+"), t("#")]), Value::text("abc#"));
    }

    /// Each alias must behave exactly as the function it names, including
    /// which of its arguments take arrays.
    #[test]
    fn the_aliases_match_the_functions_they_name() {
        for (alias_name, base_name) in [
            ("MROUD", "MROUND"),
            ("SERCH", "SEARCH"),
            ("SERCHB", "SEARCHB"),
            ("STEDV", "STDEV"),
            ("PRODUT", "PRODUCT"),
            ("TEXJOIN", "TEXTJOIN"),
            ("DDD", "DAYS360"),
            ("DEVDQ", "DEVSQ"),
            ("ODDFRRICE", "ODDFPRICE"),
        ] {
            let a = crate::lookup(alias_name).expect(alias_name);
            let b = crate::lookup(base_name).expect(base_name);
            assert_eq!(a.min_args, b.min_args, "{alias_name} arity");
            assert_eq!(a.max_args, b.max_args, "{alias_name} arity");
            assert_eq!(a.scalar_args, b.scalar_args, "{alias_name} argument lifting");
            assert_eq!(a.category, b.category, "{alias_name} category");
        }
    }

    #[test]
    fn the_aliases_are_the_functions_they_name() {
        // WPS's own names for Excel's functions.
        assert_eq!(ev("MROUD", vec![n(10.0), n(3.0)]), ev("MROUND", vec![n(10.0), n(3.0)]));
        assert_eq!(ev("MROUD", vec![n(10.0), n(3.0)]), Value::Number(9.0));
        assert_eq!(
            ev("STEDV", vec![av(col(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]))]),
            ev("STDEV", vec![av(col(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]))])
        );
        assert_eq!(ev("PRODUT", vec![n(3.0), n(4.0)]), Value::Number(12.0));
        assert_eq!(ev("TEXJOIN", vec![t("-"), b(true), t("a"), t("b")]), Value::text("a-b"));
        assert_eq!(ev("SERCH", vec![t("b"), t("abc")]), Value::Number(2.0));
    }

    #[test]
    fn widechar_makes_the_text_full_width() {
        assert_eq!(ev("WIDECHAR", vec![t("ABC")]), Value::text("ＡＢＣ"));
        assert_eq!(ev("WIDECHAR", vec![t("a1")]), Value::text("ａ１"));
        // Half-width katakana becomes full-width katakana.
        assert_eq!(ev("WIDECHAR", vec![t("\u{FF76}")]), Value::text("ザ"));
        assert_eq!(ev("WIDECHAR", vec![t("\u{FF61}")]), Value::text("ァ"));
        // A full-width character is left alone.
        assert_eq!(ev("WIDECHAR", vec![t("中")]), Value::text("中"));
    }

    #[test]
    fn the_service_backed_ones_give_the_error() {
        assert_eq!(ev("WPSAI.CLASSIFY", vec![t("a"), t("b")]), Value::Error(CellError::Value));
        assert_eq!(ev("FIELDVALUE", vec![t("close")]), Value::Error(CellError::Value));
    }
}
