//! Formula text as stored in files.
//!
//! Files hold formulas in canonical English syntax, with a few twists:
//! functions added after Excel 2007 carry an `_xlfn.` (or `_xlfn._xlws.`)
//! prefix, the parameters of `LET` / `LAMBDA` carry `_xlpm.`, the newer
//! operators are spelled as functions (`A1#` is `_xlfn.ANCHORARRAY(A1)`,
//! `@x` is `_xlfn.SINGLE(x)`) and structured references are written in
//! full (`Table1[[#This Row],[Col]]`, never `[@Col]`).

use opensheet_engine::ast::{Expr, TableCols, TableRows};
use opensheet_engine::functions::{self, Kind};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Content, ErrorKind, Locale, Workbook};
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
        // Several sheets or areas are several cells.
        Expr::Ref3D(_) | Expr::Union(_) | Expr::Intersect(..) => true,
        // One cell only for one column of the formula's own row.
        Expr::Table(t) => !(t.rows == TableRows::ThisRow && matches!(t.cols, TableCols::One(_))),
        Expr::Array(rows) => rows.len() > 1 || rows.first().is_some_and(|row| row.len() > 1),
        // What a name stands for decides; array use inside its definition is
        // the definition's business, not this formula's.
        Expr::Name(name) => match wb.defined_name(name) {
            Some(definition) if depth < 16 => scan_arrays(definition, wb, depth + 1, &mut false),
            _ => false,
        },
        Expr::SheetName(sheet, name) => {
            let definition = wb
                .sheet_index(sheet)
                .and_then(|from| wb.lookup_name(from, name));
            match definition {
                Some((_, def)) if depth < 16 => scan_arrays(&def.expr, wb, depth + 1, &mut false),
                _ => false,
            }
        }
        // The `#` and `@` operators only exist in dynamic array formulas.
        Expr::Spill(a) => {
            scan_arrays(a, wb, depth, needs);
            *needs = true;
            true
        }
        Expr::Implicit(a) => {
            scan_arrays(a, wb, depth, needs);
            *needs = true;
            false
        }
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

/// Formula text (no leading `=`) as it is stored in a file.
///
/// Besides the prefixes, references to sheets the workbook does not have
/// (left behind by a deleted sheet) are written as `#REF!`, which is what
/// they evaluate to: Excel refuses files whose formulas name unknown sheets.
/// The same goes for structured references to tables that do not exist.
pub(crate) fn to_file_text(expr: &Expr, wb: &Workbook) -> String {
    let missing = |sheet: &str| wb.sheet_index(sheet).is_none();
    let dangling = |e: &Expr| match e {
        Expr::Ref(r) => r.sheet.as_deref().is_some_and(missing),
        Expr::Ref3D(r) => missing(&r.first) || missing(&r.last),
        Expr::SheetName(sheet, _) | Expr::RefError(sheet) => missing(sheet),
        // Files always name the table.
        Expr::Table(t) => t
            .table
            .as_deref()
            .is_none_or(|name| wb.table_by_name(name).is_none()),
        _ => false,
    };
    let mut has_future = false;
    let mut has_dangling = false;
    expr.walk(&mut |e| match e {
        Expr::Func(name, _) => has_future |= future_prefix(name).is_some(),
        e => has_dangling |= dangling(e),
    });
    if !has_future && !has_dangling {
        return expr.to_file_text();
    }
    let params = if has_future {
        expr.bound_names()
    } else {
        Vec::new()
    };
    expr.map(&mut |node| match node {
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
    .to_file_text()
}

/// Whether a defined name is one Excel accepts (see the engine's
/// `names::is_valid_name`). Files with other names are reported as damaged.
pub(crate) fn is_valid_name(name: &str) -> bool {
    opensheet_engine::names::is_valid_name(name, Locale::en())
}

/// Bracket nesting depth outside string literals.
fn nesting(text: &str) -> usize {
    let mut depth = 0usize;
    let mut max = 0usize;
    let mut in_string = false;
    for b in text.bytes() {
        match b {
            b'"' => in_string = !in_string,
            b'(' | b'{' | b'[' if !in_string => {
                depth += 1;
                max = max.max(depth);
            }
            b')' | b'}' | b']' if !in_string => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Parse formula text from a file. `None` when the engine cannot represent
/// the formula (links to other workbooks, ...).
///
/// The parser itself reads the file spellings of the operators
/// (`_xlfn.ANCHORARRAY(A1)` is `A1#`, `_xlfn.SINGLE(x)` is `@x`) and the
/// long form of structured references.
pub(crate) fn from_file_text(text: &str) -> Option<Expr> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    if text.is_empty() || text.len() > MAX_FORMULA_LEN || nesting(text) > MAX_NESTING {
        return None;
    }
    let expr = parse_formula(text, Locale::en()).ok()?;
    if !text.to_ascii_uppercase().contains("_XLPM.") {
        return Some(expr);
    }
    Some(expr.map(&mut |node| match node {
        // `LET` / `LAMBDA` parameter.
        Expr::Name(n) if n.len() > 6 && n.as_bytes()[..6].eq_ignore_ascii_case(b"_xlpm.") => {
            Expr::Name(Arc::from(&n[6..]))
        }
        other => other,
    }))
}

/// After loading: a formula with a structured reference to a table the
/// workbook does not have cannot be calculated. Like the formulas that do
/// not parse, such cells keep their cached result as a constant.
pub(crate) fn keep_cached_results(wb: &mut Workbook) {
    let mut lost: Vec<(usize, u32, u32)> = Vec::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        for ((row, col), cell) in sheet.cells.iter() {
            let Content::Formula(expr) = &cell.content else {
                continue;
            };
            let mut missing = false;
            expr.walk(&mut |e| {
                if let Expr::Table(t) = e {
                    missing |= match &t.table {
                        Some(name) => wb.table_by_name(name).is_none(),
                        None => wb.table_at(si, *row, *col).is_none(),
                    };
                }
            });
            if missing {
                lost.push((si, *row, *col));
            }
        }
    }
    for (si, row, col) in lost {
        if let Some(cell) = wb.sheets[si].cells.get_mut(&(row, col)) {
            cell.content = if cell.value.is_empty() {
                Content::Empty
            } else {
                Content::Value(cell.value.clone())
            };
        }
    }
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
        assert_eq!(shown("_xlfn.SINGLE(A1:A3)*2"), "@A1:A3*2");
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

    /// A workbook with the sheets Sheet1, "My Data" and Sheet3 and the
    /// table "Sales" (Item, Qty, Unit Price) on Sheet1!A1:C4. Only what
    /// references need is filled in: tables hold more than that.
    #[allow(clippy::needless_update)]
    fn with_table() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        wb.add_sheet(2);
        assert!(wb.rename_sheet(1, "My Data"));
        let columns = ["Item", "Qty", "Unit Price"];
        wb.sheets[0].tables.push(opensheet_engine::table::Table {
            id: 1,
            name: "Sales".into(),
            range: opensheet_engine::Rect::new(0, 0, 3, 2),
            header_row: true,
            columns: columns
                .iter()
                .map(|name| opensheet_engine::table::TableColumn {
                    name: name.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        wb
    }

    #[test]
    fn references_on_save_and_load() {
        let wb = with_table();
        // (formula bar, file)
        let cases = [
            ("A1#", "_xlfn.ANCHORARRAY(A1)"),
            (
                "SUM(Sheet3!$B$2#)+'My Data'!A1#",
                "SUM(_xlfn.ANCHORARRAY(Sheet3!$B$2))+_xlfn.ANCHORARRAY('My Data'!A1)",
            ),
            ("SUM(Sheet1:Sheet3!A1:B2)", "SUM(Sheet1:Sheet3!A1:B2)"),
            (
                "AVERAGE('Sheet1:My Data'!$A:$A)",
                "AVERAGE('Sheet1:My Data'!$A:$A)",
            ),
            ("SUM((A1:A3,C1:C3))", "SUM((A1:A3,C1:C3))"),
            ("SUM(A1:C3 B2:D4)", "SUM(A1:C3 B2:D4)"),
            ("@A1:A3", "_xlfn.SINGLE(A1:A3)"),
            (
                "@(A1:A3*2)+@XLOOKUP(1,A:A,B:B)",
                "_xlfn.SINGLE((A1:A3*2))+_xlfn.SINGLE(_xlfn.XLOOKUP(1,A:A,B:B))",
            ),
            ("SUM(Sales[Qty])", "SUM(Sales[Qty])"),
            ("Sales[@Qty]", "Sales[[#This Row],[Qty]]"),
            (
                "Sales[@Qty]*Sales[@[Unit Price]]",
                "Sales[[#This Row],[Qty]]*Sales[[#This Row],[Unit Price]]",
            ),
            (
                "Sales[@[Qty]:[Unit Price]]",
                "Sales[[#This Row],[Qty]:[Unit Price]]",
            ),
            ("Sales[@]", "Sales[#This Row]"),
            (
                "Sales[[#Headers],[#Data],[Item]:[Qty]]",
                "Sales[[#Headers],[#Data],[Item]:[Qty]]",
            ),
            (
                "COUNTA(Sales[#All])+ROWS(Sales)",
                "COUNTA(Sales[#All])+ROWS(Sales)",
            ),
            ("Sheet1!Rate*'My Data'!Rate", "Sheet1!Rate*'My Data'!Rate"),
            ("Sheet3!#REF!+#N/A", "Sheet3!#REF!+#N/A"),
            (
                "SORT(UNIQUE(Sales[Item]))",
                "_xlfn._xlws.SORT(_xlfn.UNIQUE(Sales[Item]))",
            ),
        ];
        for (shown_as, stored) in cases {
            let expr = parse_formula(shown_as, Locale::en()).unwrap();
            assert_eq!(to_file_text(&expr, &wb), stored, "{shown_as}");
            let loaded = from_file_text(stored).unwrap_or_else(|| panic!("{stored} does not load"));
            assert_eq!(loaded, expr, "{stored}");
            assert_eq!(loaded.to_text(Locale::en()), shown_as);
        }
        // French formulas are stored in the same English.
        let fr = Locale::fr();
        let expr = parse_formula(
            "SOMME(Sales[[#Tout];[Qty]])+Sales[@Qty]+SOMME((A1;B2) C:C)+A1#",
            fr,
        )
        .unwrap();
        let stored = to_file_text(&expr, &wb);
        assert_eq!(
            stored,
            "SUM(Sales[[#All],[Qty]])+Sales[[#This Row],[Qty]]+SUM((A1,B2) C:C)+_xlfn.ANCHORARRAY(A1)"
        );
        assert_eq!(
            from_file_text(&stored).unwrap().to_text(fr),
            "SOMME(Sales[[#Tout];[Qty]])+Sales[@Qty]+SOMME((A1;B2) C:C)+A1#"
        );
        // What Excel writes that the formula bar never shows.
        assert_eq!(
            shown("=#REF!A1+SUM(#REF!$A$1:$B$2)"),
            "#REF!A1+SUM(#REF!$A$1:$B$2)"
        );
        assert_eq!(
            shown("Sales[[#This Row],[Unit Price]]*_xlfn.SINGLE(Sales[Qty])"),
            "Sales[@[Unit Price]]*@Sales[Qty]"
        );
        assert_eq!(shown("Sales[]"), "Sales");
    }

    #[test]
    fn references_to_what_is_gone() {
        let wb = with_table();
        let stored = |src: &str| to_file_text(&parse_formula(src, Locale::en()).unwrap(), &wb);
        // A deleted sheet: Excel refuses files naming unknown sheets, and a
        // reference to cells on no sheet says no more than `#REF!`.
        assert_eq!(
            stored("#REF!A1+SUM(#REF!A:A,Gone!B2)"),
            "#REF!+SUM(#REF!,#REF!)"
        );
        assert_eq!(
            stored("SUM(Sheet1:Gone!A1)+SUM(Gone:Away!A1)+SUM(Sheet1:Sheet3!A1)"),
            "SUM(#REF!)+SUM(#REF!)+SUM(Sheet1:Sheet3!A1)"
        );
        assert_eq!(
            stored("Gone!Rate+Gone!#REF!+Sheet3!Rate"),
            "#REF!+#REF!+Sheet3!Rate"
        );
        assert_eq!(stored("_xlfn.ANCHORARRAY(Gone!A1)"), "#REF!");
        // A table that does not exist, or that is not named.
        assert_eq!(
            stored("SUM(Orders[Qty])+Orders[@Qty]+SUM(Sales[Qty])"),
            "SUM(#REF!)+#REF!+SUM(Sales[Qty])"
        );
        assert_eq!(stored("[@Qty]*2"), "#REF!*2");
    }

    #[test]
    fn cached_results_stand_in_for_unknown_tables() {
        let mut wb = with_table();
        let cached = |text: &str, value: f64| opensheet_engine::Cell {
            content: Content::Formula(Arc::new(from_file_text(text).unwrap())),
            value: opensheet_engine::Value::Number(value),
            style: Default::default(),
        };
        wb.sheets[0]
            .cells
            .insert((5, 5), cached("SUM(Sales[Qty])", 1.0));
        wb.sheets[0]
            .cells
            .insert((6, 5), cached("SUM(Orders[Qty])+1", 2.0));
        wb.sheets[1]
            .cells
            .insert((0, 0), cached("Orders[[#This Row],[Qty]]", 3.0));
        wb.sheets[1].cells.insert((1, 0), cached("A1+1", 4.0));
        keep_cached_results(&mut wb);
        let content =
            |sheet: usize, row: u32, col: u32| wb.sheets[sheet].cells[&(row, col)].content.clone();
        assert!(matches!(content(0, 5, 5), Content::Formula(_)));
        assert_eq!(
            content(0, 6, 5),
            Content::Value(opensheet_engine::Value::Number(2.0))
        );
        assert_eq!(
            content(1, 0, 0),
            Content::Value(opensheet_engine::Value::Number(3.0))
        );
        assert!(matches!(content(1, 1, 0), Content::Formula(_)));
    }

    #[test]
    fn array_formulas_with_references() {
        let mut wb = with_table();
        wb.define_name(
            "Both",
            parse_formula("Sheet1!$A$1:$A$2,Sheet1!$C$1:$C$2", Locale::en()).unwrap(),
        );
        let needs =
            |src: &str| needs_array_evaluation(&parse_formula(src, Locale::en()).unwrap(), &wb);
        for src in [
            "SUM(Sheet1:Sheet3!A1)",
            "SUM((A1:A3,C1:C3))+COUNT(A1:C3 B2:D4)",
            "SUM(Both)+AREAS(Both)",
            "SUM(Sales[Qty])",
            "Sales[@Qty]*2",
            "Sales[@Qty]*Sales[@[Unit Price]]",
            "INDEX((A1:B2,D1:E2),1,1,2)",
            "Sheet3!#REF!+1",
        ] {
            assert!(!needs(src), "{src}");
        }
        for src in [
            "A1#",
            "SUM(A1#)",
            "@A1:A3",
            "Sales[Qty]*2",
            "SUM(Sales[Qty]*Sales[Unit Price])",
            "SUM(ABS(Sales[@]))",
            "SUM(ABS((A1:A3,C1:C3)))",
        ] {
            assert!(needs(src), "{src}");
        }
    }

    #[test]
    fn unparseable() {
        assert!(from_file_text("").is_none());
        assert!(from_file_text("[1]Sheet1!A1+1").is_none());
        assert!(from_file_text("Table1[[#This Row],[Qty]*2").is_none());
        assert!(from_file_text("SUM(Sheet1:Sheet3!Name)").is_none());
        assert!(from_file_text(&format!("{}1{}", "(".repeat(5000), ")".repeat(5000))).is_none());
    }
}
