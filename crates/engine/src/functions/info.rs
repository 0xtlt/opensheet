//! Information functions.

use super::{Category, FunctionDef, Kind};
use crate::ast::{Expr, write_sheet_name};
use crate::eval::EvalCtx;
use crate::refs::col_to_letters;
use crate::style::HAlign;
use crate::value::{Array, ErrorKind, R, Value};

// ---- IS* predicates ------------------------------------------------------
//
// These are `Kind::Scalar`, so the dispatcher applies them element-wise to
// ranges and arrays. A blank cell arrives as `Value::Empty`.

fn is_test(ctx: &EvalCtx, args: &[Expr], test: fn(&Value) -> bool) -> R<Value> {
    Ok(Value::Bool(test(&ctx.eval_scalar(&args[0]))))
}

fn f_isblank(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Empty))
}

fn f_isnumber(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Number(_)))
}

fn f_istext(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Text(_)))
}

fn f_isnontext(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| !matches!(v, Value::Text(_)))
}

fn f_iserror(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Error(_)))
}

fn f_iserr(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(
        ctx,
        args,
        |v| matches!(v, Value::Error(e) if *e != ErrorKind::NA),
    )
}

fn f_isna(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Error(ErrorKind::NA)))
}

fn f_islogical(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    is_test(ctx, args, |v| matches!(v, Value::Bool(_)))
}

/// Parity of the argument truncated to an integer. Booleans are rejected,
/// as are non-numeric texts.
fn parity(ctx: &EvalCtx, e: &Expr) -> R<bool> {
    let n = match ctx.eval_scalar(e) {
        Value::Bool(_) => return Err(ErrorKind::Value),
        v => ctx.to_number(&v)?,
    };
    Ok(n.trunc().rem_euclid(2.0) == 0.0)
}

fn f_iseven(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(parity(ctx, &args[0])?))
}

fn f_isodd(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(!parity(ctx, &args[0])?))
}

fn f_isref(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(ctx.eval_areas(&args[0]).is_ok()))
}

/// `ISFORMULA(reference)`: element-wise over a multi-cell reference.
fn f_isformula(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let reference = ctx.eval_ref(&args[0])?;
    let (sheet, r0, c0) = (reference.sheet, reference.rect.r0, reference.rect.c0);
    let has_formula = |r: u32, c: u32| Value::Bool(ctx.formula_at(sheet, r, c).is_some());
    if reference.is_single() {
        return Ok(has_formula(r0, c0));
    }
    // Same shape as the values of the range (clamped to the used cells).
    let values = ctx.range_values(&reference);
    let mut out = Array::new(values.rows, values.cols);
    for r in 0..values.rows {
        for c in 0..values.cols {
            out.set(r, c, has_formula(r0 + r as u32, c0 + c as u32));
        }
    }
    Ok(Value::array(out))
}

// ---- values and types ----------------------------------------------------

fn f_na(_ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Err(ErrorKind::NA)
}

fn f_type(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(match ctx.eval(&args[0]) {
        Value::Empty | Value::Number(_) => 1.0,
        Value::Text(_) => 2.0,
        Value::Bool(_) => 4.0,
        Value::Error(_) => 16.0,
        Value::Array(_) => 64.0,
    }))
}

fn f_n(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Number(n) => Ok(Value::Number(n)),
        Value::Bool(b) => Ok(Value::Number(if b { 1.0 } else { 0.0 })),
        Value::Error(e) => Err(e),
        _ => Ok(Value::Number(0.0)),
    }
}

fn f_error_type(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Error(e) => Ok(Value::Number(e.type_number())),
        _ => Err(ErrorKind::NA),
    }
}

// ---- workbook structure --------------------------------------------------

fn f_sheet(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let arg = match args.first() {
        None | Some(Expr::Missing) => return Ok(Value::Number(ctx.sheet as f64 + 1.0)),
        Some(e) => e,
    };
    // Of a 3-D reference, the first sheet.
    if let Ok(areas) = ctx.eval_areas(arg)
        && let Some(sheet) = areas.iter().map(|area| area.sheet).min()
    {
        return Ok(Value::Number(sheet as f64 + 1.0));
    }
    // Otherwise the argument names a sheet.
    match ctx.eval_scalar(arg) {
        Value::Text(name) => match ctx.workbook().sheet_index(&name) {
            Some(i) => Ok(Value::Number(i as f64 + 1.0)),
            None => Err(ErrorKind::NA),
        },
        Value::Error(e) => Err(e),
        _ => Err(ErrorKind::NA),
    }
}

fn f_sheets(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match args.first() {
        None | Some(Expr::Missing) => Ok(Value::Number(ctx.workbook().sheets.len() as f64)),
        Some(e) => {
            // One sheet, unless the reference is 3-D (`Sheet1:Sheet3!A1`).
            let areas = ctx.eval_areas(e).map_err(|err| match err {
                ErrorKind::Value => ErrorKind::Ref,
                other => other,
            })?;
            let mut sheets: Vec<usize> = areas.iter().map(|area| area.sheet).collect();
            sheets.sort_unstable();
            sheets.dedup();
            Ok(Value::Number(sheets.len() as f64))
        }
    }
}

/// The code `CELL("format", ...)` reports for a number format.
fn cell_format_code(fmt: &str) -> String {
    if fmt.eq_ignore_ascii_case("General") || fmt == "@" {
        return "G".to_string();
    }
    let section = fmt.split(';').next().unwrap_or(fmt);
    let lower = section.to_ascii_lowercase();
    if crate::format::is_date_format(fmt) {
        let has = |s: &str| lower.contains(s);
        let code = if has("am/pm") {
            if has("ss") { "D6" } else { "D7" }
        } else if has("h") && !has("d") && !has("y") {
            if has("ss") { "D8" } else { "D9" }
        } else if has("mmm") && has("d") && has("y") {
            "D1"
        } else if has("mmm") && has("d") {
            "D2"
        } else if has("mmm") && has("y") {
            "D3"
        } else if has("y") {
            "D4"
        } else {
            "D5"
        };
        return code.to_string();
    }
    let decimals = match section.split_once('.') {
        Some((_, rest)) => rest.chars().take_while(|c| *c == '0' || *c == '#').count(),
        None => 0,
    };
    let kind = if section.contains('%') {
        "P"
    } else if lower.contains("e+") || lower.contains("e-") {
        "S"
    } else if section.contains(['$', '€', '£', '¥']) {
        "C"
    } else if section.contains(',') {
        ","
    } else if section.contains(['0', '#']) {
        "F"
    } else {
        return "G".to_string();
    };
    format!("{kind}{decimals}")
}

/// `CELL(info_type, [reference])`. Without a reference the cell holding the
/// formula is described.
fn f_cell(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let info = ctx.text(&args[0])?.to_lowercase();
    let (sheet, row, col) = match args.get(1) {
        None | Some(Expr::Missing) => (ctx.sheet, ctx.row, ctx.col),
        Some(e) => {
            let reference = ctx.eval_ref(e)?;
            (reference.sheet, reference.rect.r0, reference.rect.c0)
        }
    };
    let wb = ctx.workbook();
    let is_self = (sheet, row, col) == (ctx.sheet, ctx.row, ctx.col);
    Ok(match info.as_str() {
        "address" => {
            let mut out = String::new();
            // Qualified with the sheet name only when it is another sheet.
            if let Some(sh) = wb.sheets.get(sheet).filter(|_| sheet != ctx.sheet) {
                write_sheet_name(&sh.name, &mut out);
                out.push('!');
            }
            out.push_str(&format!("${}${}", col_to_letters(col), row + 1));
            Value::from(out)
        }
        "row" => Value::Number(row as f64 + 1.0),
        "col" => Value::Number(col as f64 + 1.0),
        "contents" => match ctx.cell_value(sheet, row, col) {
            Value::Empty => Value::Number(0.0),
            v => v,
        },
        "type" => {
            // The describing cell itself holds a formula, hence a value.
            let code = if is_self {
                "v"
            } else {
                match ctx.cell_value(sheet, row, col) {
                    Value::Empty => "b",
                    Value::Text(_) => "l",
                    _ => "v",
                }
            };
            Value::from(code)
        }
        "format" => Value::from(cell_format_code(&wb.style_at(sheet, row, col).num_fmt)),
        "prefix" => {
            let is_text = !is_self && matches!(ctx.cell_value(sheet, row, col), Value::Text(_));
            let prefix = if !is_text {
                ""
            } else {
                match wb.style_at(sheet, row, col).h_align {
                    HAlign::General | HAlign::Left => "'",
                    HAlign::Right => "\"",
                    HAlign::Center | HAlign::CenterAcross => "^",
                    HAlign::Fill => "\\",
                    HAlign::Justify => "",
                }
            };
            Value::from(prefix)
        }
        "protect" => Value::Number(if wb.style_at(sheet, row, col).locked {
            1.0
        } else {
            0.0
        }),
        "width" => {
            // Column width in characters of the workbook's default font,
            // as the Column Width dialog counts them, rounded.
            let px = wb.sheets.get(sheet).map_or(0.0, |sh| sh.col_width(col));
            let chars = crate::metrics::col_width_in_chars(px, wb.font_metrics().digit_width);
            Value::Number(chars.round())
        }
        "color" | "parentheses" => Value::Number(0.0),
        // The workbook has no file name known to the engine.
        "filename" => Value::from(""),
        _ => return Err(ErrorKind::Value),
    })
}

/// `INFO(type_text)`: a few facts about the environment.
fn f_info(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let kind = ctx.text(&args[0])?.to_lowercase();
    Ok(match kind.as_str() {
        "numfile" => Value::Number(ctx.workbook().sheets.len() as f64),
        "recalc" => Value::from("Automatic"),
        "release" => Value::from(env!("CARGO_PKG_VERSION")),
        "system" => Value::from(if cfg!(target_os = "macos") {
            "mac"
        } else {
            "pcdos"
        }),
        "osversion" => Value::from(std::env::consts::OS),
        "origin" => Value::from("$A:$A$1"),
        "directory" => {
            let mut dir = std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !dir.is_empty() && !dir.ends_with(std::path::MAIN_SEPARATOR) {
                dir.push(std::path::MAIN_SEPARATOR);
            }
            Value::from(dir)
        }
        _ => return Err(ErrorKind::Value),
    })
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("ISBLANK", 1, 1, Kind::Scalar, Category::Information, f_isblank),
    FunctionDef::new("ISNUMBER", 1, 1, Kind::Scalar, Category::Information, f_isnumber),
    FunctionDef::new("ISTEXT", 1, 1, Kind::Scalar, Category::Information, f_istext),
    FunctionDef::new("ISNONTEXT", 1, 1, Kind::Scalar, Category::Information, f_isnontext),
    FunctionDef::new("ISERROR", 1, 1, Kind::Scalar, Category::Information, f_iserror),
    FunctionDef::new("ISERR", 1, 1, Kind::Scalar, Category::Information, f_iserr),
    FunctionDef::new("ISNA", 1, 1, Kind::Scalar, Category::Information, f_isna),
    FunctionDef::new("ISLOGICAL", 1, 1, Kind::Scalar, Category::Information, f_islogical),
    FunctionDef::new("ISEVEN", 1, 1, Kind::Scalar, Category::Information, f_iseven),
    FunctionDef::new("ISODD", 1, 1, Kind::Scalar, Category::Information, f_isodd),
    FunctionDef::new("ISREF", 1, 1, Kind::Raw, Category::Information, f_isref),
    FunctionDef::new("ISFORMULA", 1, 1, Kind::Raw, Category::Information, f_isformula),
    FunctionDef::new("NA", 0, 0, Kind::Raw, Category::Information, f_na),
    FunctionDef::new("TYPE", 1, 1, Kind::Raw, Category::Information, f_type),
    FunctionDef::new("N", 1, 1, Kind::Scalar, Category::Information, f_n),
    FunctionDef::new("ERROR.TYPE", 1, 1, Kind::Scalar, Category::Information, f_error_type),
    FunctionDef::new("SHEET", 0, 1, Kind::Raw, Category::Information, f_sheet),
    FunctionDef::new("SHEETS", 0, 1, Kind::Raw, Category::Information, f_sheets),
    FunctionDef::new("CELL", 1, 2, Kind::Raw, Category::Information, f_cell),
    FunctionDef::new("INFO", 1, 1, Kind::Scalar, Category::Information, f_info),
];

#[cfg(test)]
mod tests {
    use super::cell_format_code;
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};

    const T: Value = Value::Bool(true);
    const F: Value = Value::Bool(false);

    fn err(e: ErrorKind) -> Value {
        Value::Error(e)
    }

    #[test]
    fn is_predicates_on_literals() {
        assert_eq!(eval("ISNUMBER(1)"), T);
        assert_eq!(eval("ISNUMBER(\"1\")"), F);
        assert_eq!(eval("ISNUMBER(TRUE)"), F);
        assert_eq!(eval("ISNUMBER(DATE(2024,1,1))"), T);
        assert_eq!(eval("ISTEXT(\"a\")"), T);
        assert_eq!(eval("ISTEXT(\"\")"), T);
        assert_eq!(eval("ISTEXT(1)"), F);
        assert_eq!(eval("ISNONTEXT(1)"), T);
        assert_eq!(eval("ISNONTEXT(\"a\")"), F);
        assert_eq!(eval("ISNONTEXT(#N/A)"), T);
        assert_eq!(eval("ISLOGICAL(FALSE)"), T);
        assert_eq!(eval("ISLOGICAL(0)"), F);
        assert_eq!(eval("ISLOGICAL(1>2)"), T);
        assert_eq!(eval("ISERROR(1/0)"), T);
        assert_eq!(eval("ISERROR(#N/A)"), T);
        assert_eq!(eval("ISERROR(1)"), F);
        assert_eq!(eval("ISERR(1/0)"), T);
        assert_eq!(eval("ISERR(#N/A)"), F);
        assert_eq!(eval("ISERR(NA())"), F);
        assert_eq!(eval("ISERR(\"x\")"), F);
        assert_eq!(eval("ISNA(#N/A)"), T);
        assert_eq!(eval("ISNA(NA())"), T);
        assert_eq!(eval("ISNA(1/0)"), F);
        assert_eq!(eval("ISNA(1)"), F);
        assert_eq!(eval("ISBLANK(\"\")"), F);
        assert_eq!(eval("ISBLANK(0)"), F);
    }

    #[test]
    fn is_predicates_on_cells() {
        let w = wb(&[
            ("A1", "5"),
            ("A2", "text"),
            ("A3", "=\"\""),
            ("A5", "=1/0"),
            ("A6", "TRUE"),
            ("B1", "=ISBLANK(A4)"),
            ("B2", "=ISBLANK(A3)"),
            ("B3", "=ISBLANK(A1)"),
            ("B4", "=ISNUMBER(A1)"),
            ("B5", "=ISTEXT(A2)"),
            ("B6", "=ISNONTEXT(A4)"),
            ("B7", "=ISERROR(A5)"),
            ("B8", "=ISLOGICAL(A6)"),
            ("B9", "=ISTEXT(A3)"),
            ("B10", "=ISNUMBER(A4)"),
            // Element-wise over ranges.
            ("D1", "=ISBLANK(A1:A6)"),
            ("E1", "=ISNUMBER(A1:A6)"),
            ("F1", "=ISERROR(A1:A6)"),
            ("G1", "=SUM(IF(ISTEXT(A1:A6),1,0))"),
        ]);
        assert_eq!(val(&w, "B1"), T);
        assert_eq!(val(&w, "B2"), F);
        assert_eq!(val(&w, "B3"), F);
        assert_eq!(val(&w, "B4"), T);
        assert_eq!(val(&w, "B5"), T);
        assert_eq!(val(&w, "B6"), T);
        assert_eq!(val(&w, "B7"), T);
        assert_eq!(val(&w, "B8"), T);
        assert_eq!(val(&w, "B9"), T);
        assert_eq!(val(&w, "B10"), F);
        let column =
            |col: &str| -> Vec<Value> { (1..=6).map(|r| val(&w, &format!("{col}{r}"))).collect() };
        assert_eq!(column("D"), vec![F, F, F, T, F, F]);
        assert_eq!(column("E"), vec![T, F, F, F, F, F]);
        assert_eq!(column("F"), vec![F, F, F, F, T, F]);
        assert_eq!(val(&w, "G1"), Value::Number(2.0));
    }

    #[test]
    fn even_odd() {
        assert_eq!(eval("ISEVEN(2)"), T);
        assert_eq!(eval("ISEVEN(3)"), F);
        assert_eq!(eval("ISEVEN(2.9)"), T);
        assert_eq!(eval("ISEVEN(-1)"), F);
        assert_eq!(eval("ISEVEN(-2.5)"), T);
        assert_eq!(eval("ISEVEN(0)"), T);
        assert_eq!(eval("ISEVEN(\"4\")"), T);
        assert_eq!(eval("ISODD(3)"), T);
        assert_eq!(eval("ISODD(-3.7)"), T);
        assert_eq!(eval("ISODD(2)"), F);
        assert_eq!(eval("ISODD(0.5)"), F);
        assert_eq!(eval("ISEVEN(\"x\")"), err(ErrorKind::Value));
        assert_eq!(eval("ISODD(TRUE)"), err(ErrorKind::Value));
        assert_eq!(eval("ISODD(#N/A)"), err(ErrorKind::NA));
        let w = wb(&[
            ("A1", "1"),
            ("A2", "2"),
            ("C1", "=ISODD(A1:A2)"),
            ("D1", "=ISEVEN(B9)"),
        ]);
        assert_eq!(val(&w, "C1"), T);
        assert_eq!(val(&w, "C2"), F);
        assert_eq!(val(&w, "D1"), T);
    }

    #[test]
    fn refs_and_formulas() {
        let mut w = wb(&[
            ("A1", "1"),
            ("A2", "=A1+1"),
            ("A3", "x"),
            ("B1", "=ISREF(A1)"),
            ("B2", "=ISREF(A1:C5)"),
            ("B3", "=ISREF(1)"),
            ("B4", "=ISREF(\"A1\")"),
            ("B5", "=ISREF(A1+1)"),
            ("B6", "=ISREF(Nowhere!A1)"),
            ("C1", "=ISFORMULA(A1)"),
            ("C2", "=ISFORMULA(A2)"),
            ("C3", "=ISFORMULA(Z99)"),
            ("C4", "=ISFORMULA(5)"),
            ("C5", "=ISFORMULA(C5)"),
            ("E1", "=ISFORMULA(A1:A3)"),
            ("F1", "=ISREF((A1))"),
        ]);
        assert_eq!(val(&w, "B1"), T);
        assert_eq!(val(&w, "B2"), T);
        assert_eq!(val(&w, "B3"), F);
        assert_eq!(val(&w, "B4"), F);
        assert_eq!(val(&w, "B5"), F);
        assert_eq!(val(&w, "B6"), F);
        assert_eq!(val(&w, "C1"), F);
        assert_eq!(val(&w, "C2"), T);
        assert_eq!(val(&w, "C3"), F);
        assert_eq!(val(&w, "C4"), err(ErrorKind::Value));
        assert_eq!(val(&w, "C5"), T);
        assert_eq!(val(&w, "E1"), F);
        assert_eq!(val(&w, "E2"), T);
        assert_eq!(val(&w, "E3"), F);
        assert_eq!(val(&w, "F1"), T);
        // A defined name that stands for a range is a reference too.
        let range = crate::parser::parse_formula("A1:A3", crate::locale::Locale::en()).unwrap();
        w.define_name("Data", range);
        w.set_cell_text(0, 9, 0, "=ISREF(Data)").unwrap();
        w.set_cell_text(0, 10, 0, "=ISREF(Undefined)").unwrap();
        w.commit("names");
        assert_eq!(val(&w, "A10"), T);
        assert_eq!(val(&w, "A11"), F);
    }

    #[test]
    fn na_type_n_error_type() {
        assert_eq!(eval("NA()"), err(ErrorKind::NA));
        assert_eq!(eval("NA()+1"), err(ErrorKind::NA));
        assert_eq!(num("TYPE(1)"), 1.0);
        assert_eq!(num("TYPE(\"a\")"), 2.0);
        assert_eq!(num("TYPE(TRUE)"), 4.0);
        assert_eq!(num("TYPE(1/0)"), 16.0);
        assert_eq!(num("TYPE(#N/A)"), 16.0);
        assert_eq!(num("TYPE({1,2;3,4})"), 64.0);
        assert_eq!(num("TYPE(\"a\"&1)"), 2.0);
        assert_eq!(num("N(7)"), 7.0);
        assert_eq!(num("N(\"7\")"), 0.0);
        assert_eq!(num("N(\"abc\")"), 0.0);
        assert_eq!(num("N(TRUE)"), 1.0);
        assert_eq!(num("N(FALSE)"), 0.0);
        assert_eq!(num("N(DATE(2024,2,29))"), 45351.0);
        assert_eq!(eval("N(1/0)"), err(ErrorKind::Div0));
        assert_eq!(num("ERROR.TYPE(#NULL!)"), 1.0);
        assert_eq!(num("ERROR.TYPE(1/0)"), 2.0);
        assert_eq!(num("ERROR.TYPE(#VALUE!)"), 3.0);
        assert_eq!(num("ERROR.TYPE(#REF!)"), 4.0);
        assert_eq!(num("ERROR.TYPE(NOSUCHFN())"), 5.0);
        assert_eq!(num("ERROR.TYPE(#NUM!)"), 6.0);
        assert_eq!(num("ERROR.TYPE(NA())"), 7.0);
        assert_eq!(eval("ERROR.TYPE(1)"), err(ErrorKind::NA));
        assert_eq!(eval("ERROR.TYPE(\"x\")"), err(ErrorKind::NA));
        let w = wb(&[
            ("A1", "1"),
            ("A2", "x"),
            ("A3", "TRUE"),
            ("B1", "=TYPE(A1)"),
            ("B2", "=TYPE(A2)"),
            ("B3", "=TYPE(A3)"),
            ("B4", "=TYPE(A9)"),
            ("B5", "=TYPE(A1:A3)"),
            ("B6", "=N(A2)"),
            ("B7", "=N(A9)"),
            ("B8", "=N(A3)"),
        ]);
        assert_eq!(val(&w, "B1"), Value::Number(1.0));
        assert_eq!(val(&w, "B2"), Value::Number(2.0));
        assert_eq!(val(&w, "B3"), Value::Number(4.0));
        assert_eq!(val(&w, "B4"), Value::Number(1.0));
        assert_eq!(val(&w, "B5"), Value::Number(64.0));
        assert_eq!(val(&w, "B6"), Value::Number(0.0));
        assert_eq!(val(&w, "B7"), Value::Number(0.0));
        assert_eq!(val(&w, "B8"), Value::Number(1.0));
    }

    #[test]
    fn sheet_and_sheets() {
        let mut w = wb(&[
            ("A1", "=SHEET()"),
            ("A2", "=SHEETS()"),
            ("A3", "=SHEET(B7)"),
            ("A4", "=SHEET(\"Sheet2\")"),
            ("A5", "=SHEET(Sheet2!C3)"),
            ("A6", "=SHEET(\"Missing\")"),
            ("A7", "=SHEETS(A1:B2)"),
            ("A8", "=SHEET(Missing!A1)"),
        ]);
        let second = w.add_sheet(1);
        w.set_cell_text(second, 0, 0, "=SHEET()").unwrap();
        w.set_cell_text(second, 1, 0, "=SHEET(Sheet1!A1)").unwrap();
        w.set_cell_text(second, 2, 0, "=CELL(\"address\",Sheet1!B2)")
            .unwrap();
        w.commit("sheets");
        assert_eq!(val(&w, "A1"), Value::Number(1.0));
        assert_eq!(val(&w, "A2"), Value::Number(2.0));
        assert_eq!(val(&w, "A3"), Value::Number(1.0));
        assert_eq!(val(&w, "A4"), Value::Number(2.0));
        assert_eq!(val(&w, "A5"), Value::Number(2.0));
        assert_eq!(val(&w, "A6"), err(ErrorKind::NA));
        assert_eq!(val(&w, "A7"), Value::Number(1.0));
        assert_eq!(val(&w, "A8"), err(ErrorKind::Ref));
        assert_eq!(w.value_at(second, 0, 0), Value::Number(2.0));
        assert_eq!(w.value_at(second, 1, 0), Value::Number(1.0));
        assert_eq!(w.value_at(second, 2, 0), Value::text("Sheet1!$B$2"));
    }

    #[test]
    fn cell() {
        let w = wb(&[
            ("A1", "42"),
            ("A2", "hello"),
            ("A3", "=A1*2"),
            ("A5", "2024-03-01"),
            ("A6", "12.5%"),
            ("C1", "=CELL(\"address\",B7)"),
            ("C2", "=CELL(\"row\",B7)"),
            ("C3", "=CELL(\"col\",B7)"),
            ("C4", "=CELL(\"contents\",A1)"),
            ("C5", "=CELL(\"contents\",A2)"),
            ("C6", "=CELL(\"contents\",A3)"),
            ("C7", "=CELL(\"contents\",A4)"),
            ("C8", "=CELL(\"type\",A1)"),
            ("C9", "=CELL(\"type\",A2)"),
            ("C10", "=CELL(\"type\",A4)"),
            ("C11", "=CELL(\"TYPE\",A3)"),
            ("C12", "=CELL(\"address\",D4:F9)"),
            ("C13", "=CELL(\"row\")"),
            ("C14", "=CELL(\"col\")"),
            ("C15", "=CELL(\"address\")"),
            ("C16", "=CELL(\"nonsense\",A1)"),
            ("C17", "=CELL(\"format\",A1)"),
            ("C18", "=CELL(\"format\",A5)"),
            ("C19", "=CELL(\"format\",A6)"),
            ("C20", "=CELL(\"prefix\",A2)"),
            ("C21", "=CELL(\"prefix\",A1)"),
            ("C22", "=CELL(\"width\",A1)"),
            ("C23", "=CELL(\"address\",5)"),
            ("C24", "=CELL(\"type\")"),
            ("C25", "=CELL(\"filename\",A1)"),
            ("C26", "=CELL(\"protect\",A1)"),
        ]);
        assert_eq!(val(&w, "C1"), Value::text("$B$7"));
        assert_eq!(val(&w, "C2"), Value::Number(7.0));
        assert_eq!(val(&w, "C3"), Value::Number(2.0));
        assert_eq!(val(&w, "C4"), Value::Number(42.0));
        assert_eq!(val(&w, "C5"), Value::text("hello"));
        assert_eq!(val(&w, "C6"), Value::Number(84.0));
        assert_eq!(val(&w, "C7"), Value::Number(0.0));
        assert_eq!(val(&w, "C8"), Value::text("v"));
        assert_eq!(val(&w, "C9"), Value::text("l"));
        assert_eq!(val(&w, "C10"), Value::text("b"));
        assert_eq!(val(&w, "C11"), Value::text("v"));
        assert_eq!(val(&w, "C12"), Value::text("$D$4"));
        assert_eq!(val(&w, "C13"), Value::Number(13.0));
        assert_eq!(val(&w, "C14"), Value::Number(3.0));
        assert_eq!(val(&w, "C15"), Value::text("$C$15"));
        assert_eq!(val(&w, "C16"), err(ErrorKind::Value));
        assert_eq!(val(&w, "C17"), Value::text("G"));
        assert_eq!(val(&w, "C18"), Value::text("D4"));
        assert_eq!(val(&w, "C19"), Value::text("P2"));
        assert_eq!(val(&w, "C20"), Value::text("'"));
        assert_eq!(val(&w, "C21"), Value::text(""));
        assert_eq!(val(&w, "C22"), Value::Number(8.0));
        assert_eq!(val(&w, "C23"), err(ErrorKind::Value));
        assert_eq!(val(&w, "C24"), Value::text("v"));
        assert_eq!(val(&w, "C25"), Value::text(""));
        assert_eq!(val(&w, "C26"), Value::Number(1.0));
    }

    #[test]
    fn cell_format_codes() {
        assert_eq!(cell_format_code("General"), "G");
        assert_eq!(cell_format_code("0"), "F0");
        assert_eq!(cell_format_code("0.00"), "F2");
        assert_eq!(cell_format_code("#,##0"), ",0");
        assert_eq!(cell_format_code("#,##0.00"), ",2");
        assert_eq!(cell_format_code("$#,##0.00"), "C2");
        assert_eq!(cell_format_code("$#,##0_);($#,##0)"), "C0");
        assert_eq!(cell_format_code("0%"), "P0");
        assert_eq!(cell_format_code("0.00%"), "P2");
        assert_eq!(cell_format_code("0.00E+00"), "S2");
        assert_eq!(cell_format_code("m/d/yyyy"), "D4");
        assert_eq!(cell_format_code("d-mmm-yy"), "D1");
        assert_eq!(cell_format_code("d-mmm"), "D2");
        assert_eq!(cell_format_code("mmm-yy"), "D3");
        assert_eq!(cell_format_code("mm/dd"), "D5");
        assert_eq!(cell_format_code("h:mm AM/PM"), "D7");
        assert_eq!(cell_format_code("h:mm:ss AM/PM"), "D6");
        assert_eq!(cell_format_code("hh:mm"), "D9");
        assert_eq!(cell_format_code("hh:mm:ss"), "D8");
    }

    #[test]
    fn info() {
        let w = wb(&[
            ("A1", "=INFO(\"numfile\")"),
            ("A2", "=INFO(\"RECALC\")"),
            ("A3", "=INFO(\"system\")"),
            ("A4", "=INFO(\"release\")"),
            ("A5", "=INFO(\"bogus\")"),
            ("A6", "=INFO(\"osversion\")"),
            ("A7", "=INFO(\"origin\")"),
            ("A8", "=INFO(\"directory\")"),
            ("A9", "=INFO(1/0)"),
        ]);
        assert_eq!(val(&w, "A1"), Value::Number(1.0));
        assert_eq!(val(&w, "A2"), Value::text("Automatic"));
        assert!(matches!(val(&w, "A3"), Value::Text(s) if &*s == "mac" || &*s == "pcdos"));
        assert_eq!(val(&w, "A4"), Value::text(env!("CARGO_PKG_VERSION")));
        assert_eq!(val(&w, "A5"), err(ErrorKind::Value));
        assert!(matches!(val(&w, "A6"), Value::Text(s) if !s.is_empty()));
        assert_eq!(val(&w, "A7"), Value::text("$A:$A$1"));
        assert!(matches!(val(&w, "A8"), Value::Text(_)));
        assert_eq!(val(&w, "A9"), err(ErrorKind::Div0));
    }
}
