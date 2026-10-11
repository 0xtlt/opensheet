//! Formula text as stored in files.
//!
//! Files hold formulas in canonical English syntax, with two twists:
//! functions added after Excel 2007 carry an `_xlfn.` (or `_xlfn._xlws.`)
//! prefix, and the parameters of `LET` / `LAMBDA` carry `_xlpm.`.

use opensheet_engine::ast::Expr;
use opensheet_engine::functions::{self, Kind};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{ErrorKind, Locale, Workbook};
use std::sync::Arc;

/// Longest formula Excel accepts is 8192 characters; anything far beyond
/// that is not worth handing to a recursive parser.
const MAX_FORMULA_LEN: usize = 32_768;
/// Deepest bracket nesting handed to the (recursive) parser.
const MAX_NESTING: usize = 256;

/// The prefix a function needs in a file, if it is a "future function".
pub(crate) fn future_prefix(name: &str) -> Option<&'static str> {
    match name {
        "FILTER" | "SORT" => Some("_xlfn._xlws."),
        "ACOT"
        | "ACOTH"
        | "AGGREGATE"
        | "ANCHORARRAY"
        | "ARABIC"
        | "ARRAYTOTEXT"
        | "BASE"
        | "BETA.DIST"
        | "BETA.INV"
        | "BINOM.DIST"
        | "BINOM.DIST.RANGE"
        | "BINOM.INV"
        | "BITAND"
        | "BITLSHIFT"
        | "BITOR"
        | "BITRSHIFT"
        | "BITXOR"
        | "BYCOL"
        | "BYROW"
        | "CEILING.MATH"
        | "CEILING.PRECISE"
        | "CHISQ.DIST"
        | "CHISQ.DIST.RT"
        | "CHISQ.INV"
        | "CHISQ.INV.RT"
        | "CHISQ.TEST"
        | "CHOOSECOLS"
        | "CHOOSEROWS"
        | "COMBINA"
        | "CONCAT"
        | "CONFIDENCE.NORM"
        | "CONFIDENCE.T"
        | "COT"
        | "COTH"
        | "COVARIANCE.P"
        | "COVARIANCE.S"
        | "CSC"
        | "CSCH"
        | "DAYS"
        | "DECIMAL"
        | "DROP"
        | "ENCODEURL"
        | "ERF.PRECISE"
        | "ERFC.PRECISE"
        | "EXPAND"
        | "EXPON.DIST"
        | "F.DIST"
        | "F.DIST.RT"
        | "F.INV"
        | "F.INV.RT"
        | "F.TEST"
        | "FIELDVALUE"
        | "FILTERXML"
        | "FLOOR.MATH"
        | "FLOOR.PRECISE"
        | "FORECAST.ETS"
        | "FORECAST.ETS.CONFINT"
        | "FORECAST.ETS.SEASONALITY"
        | "FORECAST.ETS.STAT"
        | "FORECAST.LINEAR"
        | "FORMULATEXT"
        | "GAMMA"
        | "GAMMA.DIST"
        | "GAMMA.INV"
        | "GAMMALN.PRECISE"
        | "GAUSS"
        | "GROUPBY"
        | "HSTACK"
        | "HYPGEOM.DIST"
        | "IFNA"
        | "IFS"
        | "IMAGE"
        | "IMCOSH"
        | "IMCOT"
        | "IMCSC"
        | "IMCSCH"
        | "IMSEC"
        | "IMSECH"
        | "IMSINH"
        | "IMTAN"
        | "ISFORMULA"
        | "ISOMITTED"
        | "ISOWEEKNUM"
        | "LAMBDA"
        | "LET"
        | "LOGNORM.DIST"
        | "LOGNORM.INV"
        | "MAKEARRAY"
        | "MAP"
        | "MAXIFS"
        | "MINIFS"
        | "MODE.MULT"
        | "MODE.SNGL"
        | "MUNIT"
        | "NEGBINOM.DIST"
        | "NORM.DIST"
        | "NORM.INV"
        | "NORM.S.DIST"
        | "NORM.S.INV"
        | "NUMBERVALUE"
        | "PDURATION"
        | "PERCENTILE.EXC"
        | "PERCENTILE.INC"
        | "PERCENTOF"
        | "PERCENTRANK.EXC"
        | "PERCENTRANK.INC"
        | "PERMUTATIONA"
        | "PHI"
        | "PIVOTBY"
        | "POISSON.DIST"
        | "QUARTILE.EXC"
        | "QUARTILE.INC"
        | "QUERYSTRING"
        | "RANDARRAY"
        | "RANK.AVG"
        | "RANK.EQ"
        | "REDUCE"
        | "REGEXEXTRACT"
        | "REGEXREPLACE"
        | "REGEXTEST"
        | "RRI"
        | "SCAN"
        | "SEC"
        | "SECH"
        | "SEQUENCE"
        | "SHEET"
        | "SHEETS"
        | "SINGLE"
        | "SKEW.P"
        | "SORTBY"
        | "STDEV.P"
        | "STDEV.S"
        | "SWITCH"
        | "T.DIST"
        | "T.DIST.2T"
        | "T.DIST.RT"
        | "T.INV"
        | "T.INV.2T"
        | "T.TEST"
        | "TAKE"
        | "TEXTAFTER"
        | "TEXTBEFORE"
        | "TEXTJOIN"
        | "TEXTSPLIT"
        | "TOCOL"
        | "TOROW"
        | "TRIMRANGE"
        | "UNICHAR"
        | "UNICODE"
        | "UNIQUE"
        | "VALUETOTEXT"
        | "VAR.P"
        | "VAR.S"
        | "VSTACK"
        | "WEBSERVICE"
        | "WEIBULL.DIST"
        | "WRAPCOLS"
        | "WRAPROWS"
        | "XLOOKUP"
        | "XMATCH"
        | "XOR"
        | "Z.TEST" => Some("_xlfn."),
        _ => None,
    }
}

/// Functions whose result can be an array. Excel stores any formula using
/// one of them as a dynamic array formula (and shows an `@` in front of them
/// otherwise).
fn returns_array(name: &str) -> bool {
    matches!(
        name,
        "ANCHORARRAY"
            | "BYCOL"
            | "BYROW"
            | "CHOOSECOLS"
            | "CHOOSEROWS"
            | "DROP"
            | "EXPAND"
            | "FILTER"
            | "HSTACK"
            | "LAMBDA"
            | "MAKEARRAY"
            | "MAP"
            | "RANDARRAY"
            | "REDUCE"
            | "SCAN"
            | "SEQUENCE"
            | "SINGLE"
            | "SORT"
            | "SORTBY"
            | "SWITCH"
            | "TAKE"
            | "TEXTSPLIT"
            | "TOCOL"
            | "TOROW"
            | "UNIQUE"
            | "VSTACK"
            | "WRAPCOLS"
            | "WRAPROWS"
            | "XLOOKUP"
    )
}

/// Walk a formula looking for array arithmetic. Returns whether `e` itself
/// yields several values; sets `needs` when the formula has to be evaluated
/// as an array formula.
fn scan_arrays(e: &Expr, wb: &Workbook, depth: u32, needs: &mut bool) -> bool {
    match e {
        Expr::Ref(r) => !r.rect().is_single(),
        Expr::Array(rows) => rows.len() > 1 || rows.first().is_some_and(|row| row.len() > 1),
        // What a name stands for decides; array use inside its definition is
        // the definition's business, not this formula's.
        Expr::Name(name) => match wb.defined_name(name) {
            Some(definition) if depth < 16 => scan_arrays(definition, wb, depth + 1, &mut false),
            _ => false,
        },
        Expr::Paren(a) => scan_arrays(a, wb, depth, needs),
        Expr::Unary(_, a) | Expr::Percent(a) => {
            let array = scan_arrays(a, wb, depth, needs);
            *needs |= array;
            array
        }
        Expr::Binary(_, a, b) => {
            let left = scan_arrays(a, wb, depth, needs);
            let right = scan_arrays(b, wb, depth, needs);
            *needs |= left || right;
            left || right
        }
        Expr::Func(name, args) => {
            let mut any = false;
            for a in args {
                any |= scan_arrays(a, wb, depth, needs);
            }
            if returns_array(name) {
                *needs = true;
                return true;
            }
            // A function of single values is applied element by element
            // when handed a range. Other functions (aggregates, lookups)
            // take ranges as such and are assumed to return one value.
            let lifted = any && functions::lookup(name).is_some_and(|def| def.kind == Kind::Scalar);
            *needs |= lifted;
            lifted
        }
        _ => false,
    }
}

/// Whether Excel must treat the formula as a dynamic array formula to
/// compute what the engine computes.
///
/// The engine always evaluates with array semantics: `=SUM(A1:A3*B1:B3)`
/// multiplies element by element. Excel does so only for formulas stored as
/// array formulas; in a plain formula it would reduce each range to the
/// single cell on the formula's own row or column.
pub(crate) fn needs_array_evaluation(expr: &Expr, wb: &Workbook) -> bool {
    let mut needs = false;
    scan_arrays(expr, wb, 0, &mut needs);
    needs
}

/// Rebuild an expression bottom-up, passing every node through `f`.
fn rebuild(e: &Expr, f: &mut dyn FnMut(Expr) -> Expr) -> Expr {
    let node = match e {
        Expr::Unary(op, a) => Expr::Unary(*op, Box::new(rebuild(a, f))),
        Expr::Percent(a) => Expr::Percent(Box::new(rebuild(a, f))),
        Expr::Paren(a) => Expr::Paren(Box::new(rebuild(a, f))),
        Expr::Binary(op, a, b) => {
            let a = rebuild(a, f);
            let b = rebuild(b, f);
            Expr::Binary(*op, Box::new(a), Box::new(b))
        }
        Expr::Func(name, args) => {
            Expr::Func(name.clone(), args.iter().map(|a| rebuild(a, f)).collect())
        }
        Expr::Array(rows) => Expr::Array(
            rows.iter()
                .map(|row| row.iter().map(|x| rebuild(x, f)).collect())
                .collect(),
        ),
        other => other.clone(),
    };
    f(node)
}

/// Names bound by `LET(name, value, ..., body)` and
/// `LAMBDA(param, ..., body)` anywhere in the formula.
fn bound_names(expr: &Expr) -> Vec<Arc<str>> {
    let mut names: Vec<Arc<str>> = Vec::new();
    expr.walk(&mut |e| {
        let Expr::Func(name, args) = e else {
            return;
        };
        let Some((_body, params)) = args.split_last() else {
            return;
        };
        let step = match name.as_ref() {
            "LET" => 2,
            "LAMBDA" => 1,
            _ => return,
        };
        for p in params.iter().step_by(step) {
            if let Expr::Name(n) = p
                && !names.iter().any(|x| x.eq_ignore_ascii_case(n))
            {
                names.push(n.clone());
            }
        }
    });
    names
}

/// Formula text (no leading `=`) as it is stored in a file.
///
/// Besides the prefixes, references to sheets the workbook does not have
/// (left behind by a deleted sheet) are written as `#REF!`, which is what
/// they evaluate to: Excel refuses files whose formulas name unknown sheets.
pub(crate) fn to_file_text(expr: &Expr, wb: &Workbook) -> String {
    let en = Locale::en();
    let dangling = |e: &Expr| matches!(e, Expr::Ref(r) if r.sheet.as_deref().is_some_and(|s| wb.sheet_index(s).is_none()));
    let mut has_future = false;
    let mut has_dangling = false;
    expr.walk(&mut |e| match e {
        Expr::Func(name, _) => has_future |= future_prefix(name).is_some(),
        e => has_dangling |= dangling(e),
    });
    if !has_future && !has_dangling {
        return expr.to_text(en);
    }
    let params = if has_future {
        bound_names(expr)
    } else {
        Vec::new()
    };
    rebuild(expr, &mut |node| match node {
        Expr::Func(name, args) => match future_prefix(&name) {
            Some(prefix) => Expr::Func(Arc::from(format!("{prefix}{name}")), args),
            None => Expr::Func(name, args),
        },
        Expr::Name(n) if params.iter().any(|p| p.eq_ignore_ascii_case(&n)) => {
            Expr::Name(Arc::from(format!("_xlpm.{n}")))
        }
        e if dangling(&e) => Expr::Error(ErrorKind::Ref),
        other => other,
    })
    .to_text(en)
}

/// Whether a defined name is one Excel accepts: it starts with a letter,
/// `_` or `\`, continues with letters, digits, `_`, `.`, `?` or `\`, and
/// cannot be mistaken for a cell reference. Files with other names are
/// reported as damaged.
pub(crate) fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '\\');
    starts_well
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '?' | '\\'))
        && name.chars().count() <= 255
        && opensheet_engine::refs::parse_cell_name(name).is_none()
        && !is_r1c1_reference(name)
        && !name.eq_ignore_ascii_case("TRUE")
        && !name.eq_ignore_ascii_case("FALSE")
}

/// "R", "C", "RC", "R12", "C3", "R1C1": references in Excel's other
/// notation, which names may not look like either.
fn is_r1c1_reference(name: &str) -> bool {
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let upper = name.to_ascii_uppercase();
    let after_row = upper
        .strip_prefix('R')
        .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()));
    match (after_row, upper.strip_prefix('C')) {
        (Some(""), _) => true,
        (Some(rest), _) => rest.strip_prefix('C').is_some_and(digits),
        (None, Some(rest)) => digits(rest),
        (None, None) => false,
    }
}

/// Bracket nesting depth outside string literals.
fn nesting(text: &str) -> usize {
    let mut depth = 0usize;
    let mut max = 0usize;
    let mut in_string = false;
    for b in text.bytes() {
        match b {
            b'"' => in_string = !in_string,
            b'(' | b'{' if !in_string => {
                depth += 1;
                max = max.max(depth);
            }
            b')' | b'}' if !in_string => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Parse formula text from a file. `None` when the engine cannot represent
/// the formula (external links, structured references, ...).
pub(crate) fn from_file_text(text: &str) -> Option<Expr> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    if text.is_empty() || text.len() > MAX_FORMULA_LEN || nesting(text) > MAX_NESTING {
        return None;
    }
    let expr = parse_formula(text, Locale::en()).ok()?;
    let upper = text.to_ascii_uppercase();
    if !upper.contains("_XLPM.") && !upper.contains("SINGLE(") {
        return Some(expr);
    }
    Some(rebuild(&expr, &mut |node| match node {
        // `LET` / `LAMBDA` parameter.
        Expr::Name(n) if n.len() > 6 && n.as_bytes()[..6].eq_ignore_ascii_case(b"_xlpm.") => {
            Expr::Name(Arc::from(&n[6..]))
        }
        // `_xlfn.SINGLE(x)` is how files spell the `@x` operator.
        Expr::Func(name, mut args) if name.as_ref() == "SINGLE" && args.len() == 1 => {
            args.pop().unwrap_or(Expr::Missing)
        }
        other => other,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(src: &str) -> String {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        assert!(wb.rename_sheet(1, "My Data"));
        to_file_text(&parse_formula(src, Locale::en()).unwrap(), &wb)
    }

    fn shown(src: &str) -> String {
        from_file_text(src).unwrap().to_text(Locale::en())
    }

    #[test]
    fn prefixes_on_save() {
        assert_eq!(file("SUM(A1:A3)+1"), "SUM(A1:A3)+1");
        assert_eq!(file("XLOOKUP(A1,B:B,C:C)"), "_xlfn.XLOOKUP(A1,B:B,C:C)");
        assert_eq!(
            file("IF(A1,IFS(B1,1),CONCAT(\"a\",TEXTJOIN(\",\",TRUE,A:A)))"),
            "IF(A1,_xlfn.IFS(B1,1),_xlfn.CONCAT(\"a\",_xlfn.TEXTJOIN(\",\",TRUE,A:A)))"
        );
        assert_eq!(
            file("SORT(FILTER(A1:A9,B1:B9>0))"),
            "_xlfn._xlws.SORT(_xlfn._xlws.FILTER(A1:A9,B1:B9>0))"
        );
        assert_eq!(
            file("STDEV.S(A:A)+VAR.P(A:A)"),
            "_xlfn.STDEV.S(A:A)+_xlfn.VAR.P(A:A)"
        );
        assert_eq!(
            file("UNIQUE(SEQUENCE(3))"),
            "_xlfn.UNIQUE(_xlfn.SEQUENCE(3))"
        );
        assert_eq!(
            file("LET(x,1,y,x+1,x*y)"),
            "_xlfn.LET(_xlpm.x,1,_xlpm.y,_xlpm.x+1,_xlpm.x*_xlpm.y)"
        );
        assert_eq!(
            file("MAP(A1:A3,LAMBDA(v,v*rate))"),
            "_xlfn.MAP(A1:A3,_xlfn.LAMBDA(_xlpm.v,_xlpm.v*rate))"
        );
    }

    #[test]
    fn references_to_missing_sheets() {
        assert_eq!(
            file("Sheet1!A1+'my data'!B2:C3"),
            "Sheet1!A1+'my data'!B2:C3"
        );
        assert_eq!(
            file("Gone!A1+SUM('Also gone'!A:A,Sheet1!A1)"),
            "#REF!+SUM(#REF!,Sheet1!A1)"
        );
        assert_eq!(
            file("XLOOKUP(1,Gone!A:A,Sheet1!B:B)"),
            "_xlfn.XLOOKUP(1,#REF!,Sheet1!B:B)"
        );
    }

    #[test]
    fn names() {
        for name in [
            "Rate", "_x", "\\x", "Tax.2024", "été", "A1B", "ABCD1", "R2D2", "Cost", "Rc1x", "Row",
        ] {
            assert!(is_valid_name(name), "{name}");
        }
        for name in [
            "",
            "A1",
            "$A$1",
            "XFD1048576",
            "2x",
            "my name",
            "a-b",
            "R",
            "c",
            "rc",
            "R1C1",
            "C12",
            "r5c",
            "true",
            "a!b",
        ] {
            assert!(!is_valid_name(name), "{name}");
        }
    }

    #[test]
    fn prefixes_on_load() {
        assert_eq!(shown("_xlfn.XLOOKUP(A1,B:B,C:C)"), "XLOOKUP(A1,B:B,C:C)");
        assert_eq!(shown("=_xlfn._xlws.SORT(A1:A9)"), "SORT(A1:A9)");
        assert_eq!(shown("_xlfn.LET(_xlpm.x,1,_xlpm.x+1)"), "LET(x,1,x+1)");
        assert_eq!(shown("_xlfn.SINGLE(A1:A3)*2"), "A1:A3*2");
        assert_eq!(shown("\"_xlpm.\"&A1"), "\"_xlpm.\"&A1");
    }

    #[test]
    fn array_formulas() {
        let mut wb = Workbook::new(Locale::en());
        wb.define_name("Rate", parse_formula("0.2", Locale::en()).unwrap());
        wb.define_name("Cell", parse_formula("Sheet1!$B$2", Locale::en()).unwrap());
        wb.define_name(
            "Prices",
            parse_formula("Sheet1!$B$2:$B$9", Locale::en()).unwrap(),
        );
        let needs =
            |src: &str| needs_array_evaluation(&parse_formula(src, Locale::en()).unwrap(), &wb);
        // Plain formulas.
        for src in [
            "A1+B1*2",
            "SUM(A1:A3)+MAX(B:B)-A1%",
            "IF(A1>0,\"yes\",\"no\")",
            "A1*Rate+Cell",
            "SUM(Prices)",
            "ABS(-A1)",
            "SUM({1,2;3,4})+{5}",
            "VLOOKUP(A1,C1:D9,2,FALSE)",
            "NOSUCHFUNCTION(A1:A3)",
        ] {
            assert!(!needs(src), "{src}");
        }
        // Array arithmetic and functions returning arrays.
        for src in [
            "A1:A3*2",
            "SUM(A1:A3*B1:B3)",
            "SUM((A1:A9>5)*(B1:B9))",
            "MAX(IF(A1:A9=\"x\",B1:B9))",
            "SUM(-A:A)",
            "SUM(Prices*Rate)",
            "SUM(ABS(A1:A3))",
            "SUM(ABS({1,-2,3}))",
            "{1,2,3}+1",
            "XLOOKUP(A1,B:B,C:C)",
            "SUM(FILTER(A1:A9,B1:B9>0))",
            "IF(A1,SORT(B1:B9),0)",
        ] {
            assert!(needs(src), "{src}");
        }
    }

    #[test]
    fn unparseable() {
        assert!(from_file_text("").is_none());
        assert!(from_file_text("[1]Sheet1!A1+1").is_none());
        assert!(from_file_text("Table1[[#This Row],[Qty]]*2").is_none());
        assert!(from_file_text(&format!("{}1{}", "(".repeat(5000), ")".repeat(5000))).is_none());
    }
}
