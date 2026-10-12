//! Differential test of recalculation, shared by `tests/recalc_diff.rs` and
//! `examples/bench.rs`.
//!
//! Two workbooks receive the same random edits. One recalculates what
//! depends on each edit; the other is told to forget what it knows and
//! calculates every formula every time. After each step they must hold the
//! same values, the same spilled arrays and the same circular-reference
//! flag. Both also check that formulas read nothing but the precedents the
//! dependency graph knows them by (`deps::set_audit`).
//!
//! The workbooks are small and dense so that formulas keep running into
//! one another: ranges over formulas, other sheets, names, a table, arrays
//! that spill and get blocked, volatile functions, errors, circular
//! references.

use opensheet_engine::filter::{Criteria, FilterScope, Top10, ValueFilter};
use opensheet_engine::names::NameScope;
use opensheet_engine::ops::{PasteMode, SortLevel, SortSpec};
use opensheet_engine::refs::cell_name;
use opensheet_engine::table::TableOption;
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{CalcMode, Content, IterativeCalc, Locale, Rect, Value, Workbook};

const ROWS: u32 = 9;
const COLS: u32 = 6;

/// SplitMix64: small, and the same everywhere.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5DEE_CE66_D1CE_4E5B)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }

    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len() as u32) as usize]
    }
}

/// What is done to the table of the workbook (the first one found), or to
/// make one. Places are counted from the table's corner.
#[derive(Clone, Debug)]
pub enum TableOp {
    Add(usize, Rect),
    /// More or fewer rows and columns.
    Resize(i32, i32),
    InsertRows(u32, u32),
    DeleteRows(u32, u32),
    InsertCols(u32, u32),
    DeleteCols(u32, u32),
    Totals(bool),
    Headers(bool),
    Rename(String),
    ToRange,
    Delete,
}

/// What is done to the filter of a sheet, or of the table when the
/// column is negative.
#[derive(Clone, Debug)]
pub enum FilterOp {
    Set(Rect),
    /// Keep the rows showing one of these values in a column.
    Values(u32, Vec<String>, bool),
    Top(u32, bool, u32),
    Clear,
    Remove,
    Reapply,
}

/// One step. Everything random is decided when the step is drawn, so that
/// both workbooks do exactly the same thing.
#[derive(Clone, Debug)]
pub enum Op {
    Set(usize, u32, u32, String),
    /// Several cells in one transaction.
    Batch(Vec<(usize, u32, u32, String)>),
    /// Cells typed, then the transaction dropped.
    Rollback(Vec<(usize, u32, u32, String)>),
    /// Contents, everything, formats, comments.
    Clear(usize, Rect, u8),
    /// A sort on several keys, perhaps left to right.
    SortRange(usize, Rect, Vec<(u32, bool)>, bool, bool),
    RemoveDuplicates(usize, Rect, bool),
    Table(TableOp),
    Filter(usize, FilterOp),
    Note(usize, u32, u32),
    Paste(usize, Rect, usize, Rect, u8),
    PasteText(usize, u32, u32, String),
    FillDown(usize, Rect),
    FillRight(usize, Rect),
    Autofill(usize, Rect, Rect),
    InsertRows(usize, u32, u32),
    DeleteRows(usize, u32, u32),
    InsertCols(usize, u32, u32),
    DeleteCols(usize, u32, u32),
    Sort(usize, Rect, u32, bool),
    Move(usize, Rect, usize, u32, u32),
    Merge(usize, Rect),
    Undo(u32),
    Redo(u32),
    /// Rows hidden through the workbook (an undoable transaction).
    HideRows(usize, u32, u32, bool),
    /// Rows hidden behind its back, then announced.
    HideDirect(usize, u32, bool),
    DefineName(String, String),
    DeleteName(String),
    /// The name of sheet scope is given another definition.
    DefineLocal(usize, String),
    /// The table grows or shrinks by a row, in a transaction or not.
    ResizeTable(i32, bool),
    AddSheet(usize),
    DeleteSheet(usize),
    RenameSheet(usize, String),
    MoveSheet(usize, usize),
    DuplicateSheet(usize),
    Recalc,
    /// Manual calculation: edits, perhaps "calculate sheet", then
    /// "calculate now" and back to automatic.
    Manual(Vec<(usize, u32, u32, String)>, Option<usize>),
    Iterative(bool, u32),
}

/// The cell a formula is written for. References mostly point to cells
/// before it (a sheet further left, a row further up, or further left on
/// its row), which cannot close a loop; now and then they point anywhere,
/// and circular references come and go.
struct Site<'a> {
    wb: &'a Workbook,
    sheet: usize,
    row: u32,
    col: u32,
    /// How often, in percent, a reference points anywhere.
    wild: u32,
}

const WILD: u32 = 2;

fn any_cell(rng: &mut Rng) -> (u32, u32) {
    (rng.below(ROWS), rng.below(COLS))
}

fn rect(rng: &mut Rng) -> Rect {
    let (r0, c0) = any_cell(rng);
    Rect::new(
        r0,
        c0,
        (r0 + rng.below(3)).min(ROWS - 1),
        (c0 + rng.below(3)).min(COLS - 1),
    )
}

fn dollars(rng: &mut Rng, row: u32, col: u32) -> String {
    let name = cell_name(row, col);
    let letters = name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    match rng.below(8) {
        0 => format!("${name}"),
        1 => format!("${}${}", &name[..letters], &name[letters..]),
        _ => name,
    }
}

impl Site<'_> {
    fn quoted(&self, sheet: usize) -> String {
        let name = &self.wb.sheets[sheet].name;
        match name.contains(' ') {
            true => format!("'{name}'"),
            false => name.clone(),
        }
    }

    /// A sheet before this one, when there is one.
    fn lower(&self, rng: &mut Rng) -> Option<usize> {
        if rng.chance(self.wild) {
            return Some(rng.below(self.wb.sheets.len() as u32) as usize);
        }
        (self.sheet > 0).then(|| rng.below(self.sheet as u32) as usize)
    }

    /// A cell of this sheet, before this one.
    fn near(&self, rng: &mut Rng) -> String {
        let before = self.row * COLS + self.col.min(COLS);
        if rng.chance(self.wild) || self.row >= ROWS {
            let (row, col) = any_cell(rng);
            return dollars(rng, row, col);
        }
        if before == 0 {
            return "1".to_string();
        }
        let k = rng.below(before);
        dollars(rng, k / COLS, k % COLS)
    }

    /// A small range of this sheet, before this cell.
    fn block(&self, rng: &mut Rng) -> String {
        if rng.chance(self.wild) || self.row >= ROWS {
            return rect(rng).name();
        }
        if self.row == 0 {
            if self.col == 0 {
                return "1".to_string();
            }
            let c1 = rng.below(self.col.min(COLS));
            return Rect::new(0, c1.saturating_sub(rng.below(3)), 0, c1).name();
        }
        let r1 = rng.below(self.row);
        let c0 = rng.below(COLS);
        Rect::new(
            r1.saturating_sub(rng.below(4)),
            c0,
            r1,
            (c0 + rng.below(3)).min(COLS - 1),
        )
        .name()
    }

    /// A cell before this one, on this sheet or an earlier one.
    fn cell(&self, rng: &mut Rng) -> String {
        match self.lower(rng).filter(|_| rng.chance(25)) {
            Some(sheet) => {
                let (row, col) = any_cell(rng);
                format!("{}!{}", self.quoted(sheet), dollars(rng, row, col))
            }
            None => self.near(rng),
        }
    }

    /// A range before this cell, on this sheet or an earlier one.
    fn range(&self, rng: &mut Rng) -> String {
        match self.lower(rng).filter(|_| rng.chance(25)) {
            Some(sheet) => format!("{}!{}", self.quoted(sheet), rect(rng).name()),
            None => self.block(rng),
        }
    }

    /// Whole columns or rows of an earlier sheet; a plain range on the
    /// first sheet.
    fn columns(&self, rng: &mut Rng) -> String {
        let Some(sheet) = self.lower(rng) else {
            return self.block(rng);
        };
        let whole = match rng.chance(75) {
            true => {
                let c = rng.below(COLS);
                Rect::new(
                    0,
                    c,
                    opensheet_engine::MAX_ROWS - 1,
                    (c + rng.below(2)).min(COLS - 1),
                )
            }
            false => {
                let r = rng.below(ROWS);
                Rect::new(r, 0, r + rng.below(2), opensheet_engine::MAX_COLS - 1)
            }
        };
        format!("{}!{}", self.quoted(sheet), whole.name())
    }

    fn formula(&self, rng: &mut Rng) -> String {
        let (a, b, c) = (self.cell(rng), self.cell(rng), self.cell(rng));
        let (r, q) = (self.range(rng), self.range(rng));
        // Syntax that wants plain references of one sheet.
        let (near, block, block2) = (self.near(rng), self.block(rng), self.block(rng));
        let whole = self.columns(rng);
        let n = rng.below(4) + 1;
        // The table is at the top of the first sheet: below it and on the
        // other sheets it is behind; "this row" is read from its right.
        let wild = rng.chance(self.wild);
        let below = self.sheet > 0 || self.row > 7 || wild;
        let beside = (self.sheet == 0 && self.col > 3) || wild;
        match rng.below(76) {
            0 => format!("={a}+{b}"),
            1 => format!("={a}*2-{b}"),
            2 => format!("={a}+1"),
            3 => format!("=SUM({r})"),
            4 => format!("=AVERAGE({r})+{a}"),
            5 => format!("=IF({a}>3,{b},{c})"),
            6 => format!("=COUNT({whole})"),
            7 => format!("=SUM({whole})"),
            8 => format!("=MAX({r})-MIN({q})"),
            9 => format!("=SUM({r},{a})"),
            10 => format!("={a}+{b}+{c}"),
            11 => match self.lower(rng) {
                Some(last) => format!("=SUM({}:{}!{near})", self.quoted(0), self.quoted(last)),
                None => format!("=SUM({block})"),
            },
            12 => format!("=SUM(({block},{block2}))"),
            13 => format!("=SUM({block} {block2})"),
            14 => format!("=Rate*{a}"),
            15 => "=SUM(Block)".to_string(),
            16 => "=Left+1".to_string(),
            17 => "=Twice+Local".to_string(),
            18 if below => "=SUM(Sales[Qty])".to_string(),
            19 if beside => "=Sales[@Qty]*Sales[@Price]".to_string(),
            20 if below => "=COUNTA(Sales[#All])+ROWS(Sales)".to_string(),
            21 => format!("=SEQUENCE({a})"),
            22 => format!("=SEQUENCE({n},{})", rng.below(3) + 1),
            23 => format!("={r}*2"),
            24 => format!("=TRANSPOSE({r})"),
            25 => format!("=SORT({r})"),
            26 => format!("=FILTER({r},{r}>{n})"),
            27 => format!("=UNIQUE({r})"),
            28 => format!("={near}#"),
            29 => format!("=SUM({near}#)"),
            30 => format!("=INDEX({near}#,{n})"),
            31 => format!("=NOW()*0+{a}"),
            32 => format!("=RAND()*0+{a}"),
            33 => format!("=OFFSET({near},-1,0)"),
            34 => format!("=SUM(OFFSET({near},0,0,1,2))"),
            35 => format!(
                "=INDIRECT(\"{}\"&{n})",
                ["A", "B", "C"][rng.below(3) as usize]
            ),
            36 => format!("=INDIRECT(\"{near}\")+1"),
            37 => format!("=RANDBETWEEN(2,2)+{a}"),
            38 => format!("=1/{a}"),
            39 => format!("={a}/0"),
            40 => format!("=NOSUCH({a})"),
            41 => format!("=VLOOKUP({a},{r},2,FALSE)"),
            42 => format!("=MATCH({a},{r},0)"),
            43 => format!("=IFERROR(1/{a},{b})"),
            44 => format!("=INDEX({r},{n},1)"),
            45 => format!("=XLOOKUP({a},{r},{q})"),
            46 => format!("=LOOKUP({a},{r},{b})"),
            47 => format!("=SUMIF({r},\">{n}\",{b})"),
            48 => format!("=CHOOSE({n},{a},{b},{c})"),
            49 => format!("=SUBTOTAL(9,{r})"),
            50 => format!("=SUBTOTAL(109,{r})"),
            51 => format!("=AGGREGATE(9,5,{r})"),
            52 => format!("=ISFORMULA({a})"),
            53 => format!("=FORMULATEXT({a})"),
            54 => format!("=ROWS({whole}&\"\")"),
            55 => format!("=COLUMNS({whole}&\"\")"),
            56 => format!("={a}&\"x\""),
            57 => format!("=LEN({a})+ROW()"),
            58 => format!("=COUNTA({whole})"),
            59 => format!("=SUM({near}:{near})"),
            60 => format!("=@{block}"),
            61 => format!("=SUMPRODUCT({whole}*1)"),
            62 => format!("=COUNTIF({whole},{a})"),
            63 => format!("=IF({a}=0,{b},{near}#)"),
            64 => format!("=CELL(\"contents\",{a})"),
            65 => format!("=AVERAGEIF({r},\"<>0\",{b})"),
            66 => format!("=LOOKUP({n},{whole})"),
            67 => format!("=SUM(INDEX({r},0,1))"),
            68 => format!("=TODAY()*0+SUM({r})"),
            69 => format!("=SUM({r})-SUM({q})"),
            70 => format!("=COUNTBLANK({r})"),
            71 => format!("=SUBTOTAL(103,{whole})"),
            72 => format!("=VLOOKUP({a},{whole},1,FALSE)"),
            73 => format!("=SUM(SEQUENCE({a}))"),
            74 => format!("={whole}"),
            _ => format!("=MIN({r})"),
        }
    }
}

fn entry(rng: &mut Rng, site: &Site) -> String {
    match rng.below(100) {
        0..=29 => rng.below(9).to_string(),
        30..=34 => format!("{}.5", rng.below(20)),
        35..=39 => rng.pick(&["x", "y", "apple", "TRUE", ""]).to_string(),
        40..=44 => String::new(),
        _ => site.formula(rng),
    }
}

fn typed(rng: &mut Rng, wb: &Workbook) -> (usize, u32, u32, String) {
    let sheet = rng.below(wb.sheets.len() as u32) as usize;
    let (row, col) = any_cell(rng);
    let site = Site {
        wb,
        sheet,
        row,
        col,
        wild: WILD,
    };
    (sheet, row, col, entry(rng, &site))
}

fn typed_cells(rng: &mut Rng, wb: &Workbook, most: u32) -> Vec<(usize, u32, u32, String)> {
    (0..rng.below(most) + 1).map(|_| typed(rng, wb)).collect()
}

/// Draw the next step for a workbook in the state of `wb`.
pub fn draw(rng: &mut Rng, wb: &Workbook) -> Op {
    let sheets = wb.sheets.len();
    let s = rng.below(sheets as u32) as usize;
    let other = rng.below(sheets as u32) as usize;
    // A circular reference stops everything else from being exercised:
    // more often than not, step back out of it.
    if wb.circular && rng.chance(50) {
        return Op::Undo(rng.below(3) + 1);
    }
    match rng.below(1000) {
        0..=409 => {
            let (sheet, row, col, text) = typed(rng, wb);
            Op::Set(sheet, row, col, text)
        }
        410..=424 => {
            let area = rect(rng);
            let keys = (0..rng.below(2) + 1)
                .map(|_| (rng.below(3), rng.chance(50)))
                .collect();
            Op::SortRange(s, area, keys, rng.chance(40), rng.chance(20))
        }
        425..=434 => Op::RemoveDuplicates(s, rect(rng), rng.chance(40)),
        435..=459 => Op::Table(match rng.below(14) {
            0 | 1 => TableOp::Add(s, rect(rng)),
            2 | 3 => TableOp::Resize(rng.below(3) as i32 - 1, rng.below(3) as i32 - 1),
            4 | 5 => TableOp::InsertRows(rng.below(4), rng.below(2) + 1),
            6 => TableOp::DeleteRows(rng.below(4), rng.below(2) + 1),
            7 => TableOp::InsertCols(rng.below(3), 1),
            8 => TableOp::DeleteCols(rng.below(3), 1),
            9 | 10 => TableOp::Totals(rng.chance(60)),
            11 => TableOp::Headers(rng.chance(60)),
            12 => match rng.below(3) {
                0 => TableOp::ToRange,
                1 => TableOp::Delete,
                _ => TableOp::Rename(rng.pick(&["Sales", "Orders"]).to_string()),
            },
            _ => TableOp::Rename("Sales".to_string()),
        }),
        460..=474 => {
            let values = (0..rng.below(3) + 1)
                .map(|_| rng.below(9).to_string())
                .collect();
            Op::Filter(
                s,
                match rng.below(8) {
                    0 | 1 => FilterOp::Set(rect(rng)),
                    2 | 3 => FilterOp::Values(rng.below(3), values, rng.chance(30)),
                    4 => FilterOp::Top(rng.below(3), rng.chance(50), rng.below(3) + 1),
                    5 => FilterOp::Clear,
                    6 => FilterOp::Remove,
                    _ => FilterOp::Reapply,
                },
            )
        }
        475..=479 => Op::Note(s, rng.below(ROWS), rng.below(COLS)),
        480..=529 => Op::Batch(typed_cells(rng, wb, 5)),
        530..=544 => Op::Rollback(typed_cells(rng, wb, 3)),
        545..=589 => Op::Clear(s, rect(rng), [0, 0, 0, 1, 1, 2, 3][rng.below(7) as usize]),
        590..=639 => Op::Paste(s, rect(rng), other, rect(rng), rng.below(10) as u8),
        640..=649 => {
            let (row, col) = any_cell(rng);
            let above = match row {
                0 => "1".to_string(),
                _ => cell_name(row - 1, col),
            };
            let text = format!("{}\t={above}+1\n{}\tz", rng.below(9), rng.below(9));
            Op::PasteText(s, row, col, text)
        }
        650..=674 => Op::FillDown(s, rect(rng)),
        675..=694 => Op::FillRight(s, rect(rng)),
        695..=714 => {
            let src = rect(rng);
            let dest = match rng.chance(50) {
                true => Rect::new(
                    src.r0,
                    src.c0,
                    (src.r1 + 1 + rng.below(3)).min(ROWS + 2),
                    src.c1,
                ),
                false => Rect::new(
                    src.r0,
                    src.c0,
                    src.r1,
                    (src.c1 + 1 + rng.below(3)).min(COLS + 2),
                ),
            };
            Op::Autofill(s, src, dest)
        }
        715..=734 => Op::InsertRows(s, rng.below(ROWS), rng.below(2) + 1),
        735..=754 => Op::DeleteRows(s, rng.below(ROWS), rng.below(2) + 1),
        755..=769 => Op::InsertCols(s, rng.below(COLS), rng.below(2) + 1),
        770..=784 => Op::DeleteCols(s, rng.below(COLS), rng.below(2) + 1),
        785..=804 => {
            let area = rect(rng);
            Op::Sort(s, area, area.c0 + rng.below(area.cols()), rng.chance(50))
        }
        805..=814 => Op::Move(s, rect(rng), other, rng.below(ROWS), rng.below(COLS)),
        815..=819 => Op::Merge(s, rect(rng)),
        820..=879 => Op::Undo(rng.below(3) + 1),
        880..=909 => Op::Redo(rng.below(3) + 1),
        910..=924 => {
            let r0 = rng.below(ROWS);
            Op::HideRows(s, r0, (r0 + rng.below(3)).min(ROWS - 1), rng.chance(60))
        }
        925..=932 => Op::HideDirect(s, rng.below(ROWS), rng.chance(60)),
        933..=942 => {
            let name = rng.pick(&["Rate", "Block", "Twice", "Left", "Extra"]);
            let first = &wb.sheets[0].name;
            let definition = match rng.below(5) {
                0 => format!("'{first}'!{}", cell_name(rng.below(2), rng.below(COLS))),
                1 => format!(
                    "'{first}'!{}",
                    Rect::new(0, 0, rng.below(3), rng.below(3)).name()
                ),
                2 => "Rate*2".to_string(),
                3 => format!("SUM('{first}'!{})", rect(rng).name()),
                _ => (rng.below(5) + 1).to_string(),
            };
            Op::DefineName(name.to_string(), definition)
        }
        943..=944 => Op::DeleteName(rng.pick(&["Rate", "Block", "Twice", "Extra"]).to_string()),
        945..=946 => Op::DefineLocal(
            s,
            format!(
                "'{}'!{}+{}",
                wb.sheets[0].name,
                cell_name(rng.below(2), rng.below(2)),
                rng.below(3)
            ),
        ),
        947..=956 => Op::ResizeTable(if rng.chance(50) { 1 } else { -1 }, rng.chance(50)),
        957..=960 if sheets < 5 => Op::AddSheet(other),
        961..=963 if sheets > 2 => Op::DeleteSheet(s),
        964..=966 => Op::RenameSheet(s, format!("Data {}", rng.below(3))),
        967..=969 => Op::MoveSheet(s, other),
        970..=971 if sheets < 5 => Op::DuplicateSheet(s),
        972..=976 => Op::Recalc,
        977..=991 => Op::Manual(typed_cells(rng, wb, 4), rng.chance(60).then_some(s)),
        992..=996 => Op::Iterative(rng.chance(60), rng.below(6) + 1),
        _ => Op::Undo(1),
    }
}

fn type_cells(wb: &mut Workbook, cells: &[(usize, u32, u32, String)]) {
    for (sheet, row, col, text) in cells {
        // What does not parse is kept as text, as the grid does.
        if wb.set_cell_text(*sheet, *row, *col, text).is_err() {
            wb.set_content(*sheet, *row, *col, Content::Value(Value::text(text)));
        }
    }
}

fn table_step(wb: &mut Workbook, op: &TableOp) {
    let found = wb
        .sheets
        .iter()
        .enumerate()
        .find_map(|(si, sh)| sh.tables.first().map(|t| (si, t.id, t.range)));
    let Some((sheet, id, range)) = found else {
        if let TableOp::Add(sheet, rect) = op
            && let Ok(id) = wb.add_table(*sheet, *rect, true)
        {
            let _ = wb.rename_table(*sheet, id, "Sales");
        }
        return;
    };
    let grown = |by: i32, from: u32, least: u32| (from as i32 + by).max(least as i32) as u32;
    match op {
        TableOp::Add(..) => {}
        TableOp::Resize(rows, cols) => {
            let to = Rect::new(
                range.r0,
                range.c0,
                grown(*rows, range.r1, range.r0 + 1),
                grown(*cols, range.c1, range.c0),
            );
            let _ = wb.resize_table(sheet, id, to);
        }
        TableOp::InsertRows(at, count) => {
            let _ = wb.table_insert_rows(sheet, id, (range.r0 + 1 + at).min(range.r1 + 1), *count);
        }
        TableOp::DeleteRows(at, count) => {
            let _ = wb.table_delete_rows(sheet, id, (range.r0 + 1 + at).min(range.r1), *count);
        }
        TableOp::InsertCols(at, count) => {
            let _ = wb.table_insert_cols(sheet, id, (range.c0 + at).min(range.c1 + 1), *count);
        }
        TableOp::DeleteCols(at, count) => {
            let _ = wb.table_delete_cols(sheet, id, (range.c0 + at).min(range.c1), *count);
        }
        TableOp::Totals(on) => {
            let _ = wb.set_table_option(sheet, id, TableOption::TotalsRow, *on);
        }
        TableOp::Headers(on) => {
            let _ = wb.set_table_option(sheet, id, TableOption::HeaderRow, *on);
        }
        TableOp::Rename(name) => {
            let _ = wb.rename_table(sheet, id, name);
        }
        TableOp::ToRange => {
            wb.convert_table_to_range(sheet, id);
        }
        TableOp::Delete => {
            wb.delete_table(sheet, id);
        }
    }
}

fn filter_step(wb: &mut Workbook, sheet: usize, op: &FilterOp) {
    // The table's own filter when the sheet has a table and no filter.
    let scope = match (
        &wb.sheets[sheet].auto_filter,
        wb.sheets[sheet].tables.first(),
    ) {
        (None, Some(table)) => FilterScope::Table(table.id),
        _ => FilterScope::Sheet,
    };
    match op {
        FilterOp::Set(range) => {
            let _ = wb.set_auto_filter(sheet, *range);
        }
        FilterOp::Values(col, values, blanks) => {
            let wanted = ValueFilter {
                values: values.clone(),
                blanks: *blanks,
                ..ValueFilter::default()
            };
            wb.set_filter_criteria(sheet, scope, *col, Some(Criteria::Values(wanted)));
        }
        FilterOp::Top(col, top, count) => {
            let wanted = Top10::new(*top, false, *count as f64);
            wb.set_filter_criteria(sheet, scope, *col, Some(Criteria::Top10(wanted)));
        }
        FilterOp::Clear => {
            wb.clear_filters(sheet, scope);
        }
        FilterOp::Remove => {
            wb.remove_auto_filter(sheet);
        }
        FilterOp::Reapply => wb.reapply_filters(sheet),
    }
}

/// Do one step.
pub fn apply(wb: &mut Workbook, op: &Op) {
    match op {
        Op::Set(sheet, row, col, text) => {
            type_cells(wb, &[(*sheet, *row, *col, text.clone())]);
            // As the grid does: a table grows over what is typed next to
            // it, a formula fills its column.
            wb.tables_after_edit(*sheet, Rect::cell(*row, *col));
            wb.commit("type");
        }
        Op::Batch(cells) => {
            type_cells(wb, cells);
            wb.commit("type");
        }
        Op::Rollback(cells) => {
            type_cells(wb, cells);
            wb.rollback();
        }
        Op::Clear(sheet, rect, mode) => {
            let mode = [
                ClearMode::Contents,
                ClearMode::All,
                ClearMode::Formats,
                ClearMode::Comments,
            ][*mode as usize];
            wb.clear(*sheet, *rect, mode);
            wb.commit("clear");
        }
        Op::Paste(from, source, to, dest, mode) => {
            let clip = wb.copy(*from, *source);
            let mode = [
                PasteMode::All,
                PasteMode::Values,
                PasteMode::Formulas,
                PasteMode::Formats,
                PasteMode::Comments,
                PasteMode::Validation,
                PasteMode::AllExceptBorders,
                PasteMode::ColumnWidths,
                PasteMode::FormulasAndNumberFormats,
                PasteMode::ValuesAndNumberFormats,
            ][*mode as usize];
            wb.paste(*to, *dest, &clip, mode);
            wb.commit("paste");
        }
        Op::SortRange(sheet, rect, keys, header, sideways) => {
            let levels = keys
                .iter()
                .map(|(key, descending)| SortLevel::values(*key, *descending))
                .collect();
            let spec = SortSpec {
                levels,
                has_header: *header,
                left_to_right: *sideways,
                ..SortSpec::default()
            };
            let _ = wb.sort_range(*sheet, *rect, &spec);
            wb.commit("sort");
        }
        Op::RemoveDuplicates(sheet, rect, header) => {
            let _ = wb.remove_duplicates(*sheet, *rect, &[], *header);
            wb.commit("duplicates");
        }
        Op::Table(op) => {
            table_step(wb, op);
            wb.commit("table");
        }
        Op::Filter(sheet, op) => {
            filter_step(wb, *sheet, op);
            wb.commit("filter");
        }
        Op::Note(sheet, row, col) => {
            wb.set_note(*sheet, *row, *col, "fuzz", "a note");
            wb.commit("note");
        }
        Op::PasteText(sheet, row, col, text) => {
            wb.paste_text(*sheet, *row, *col, text);
            wb.commit("paste");
        }
        Op::FillDown(sheet, rect) => {
            wb.fill_down(*sheet, *rect);
            wb.commit("fill");
        }
        Op::FillRight(sheet, rect) => {
            wb.fill_right(*sheet, *rect);
            wb.commit("fill");
        }
        Op::Autofill(sheet, src, dest) => {
            wb.autofill(*sheet, *src, *dest);
            wb.commit("fill");
        }
        Op::InsertRows(sheet, at, count) => {
            wb.insert_rows(*sheet, *at, *count);
            wb.commit("insert");
        }
        Op::DeleteRows(sheet, at, count) => {
            wb.delete_rows(*sheet, *at, *count);
            wb.commit("delete");
        }
        Op::InsertCols(sheet, at, count) => {
            wb.insert_cols(*sheet, *at, *count);
            wb.commit("insert");
        }
        Op::DeleteCols(sheet, at, count) => {
            wb.delete_cols(*sheet, *at, *count);
            wb.commit("delete");
        }
        Op::Sort(sheet, rect, key, ascending) => {
            wb.sort_rows(*sheet, *rect, *key, *ascending, false);
            wb.commit("sort");
        }
        Op::Move(from, rect, to, row, col) => {
            wb.move_range(*from, *rect, *to, *row, *col);
            wb.commit("move");
        }
        Op::Merge(sheet, rect) => {
            wb.merge(*sheet, *rect);
            wb.commit("merge");
        }
        Op::Undo(times) => {
            for _ in 0..*times {
                wb.undo();
            }
        }
        Op::Redo(times) => {
            for _ in 0..*times {
                wb.redo();
            }
        }
        Op::HideRows(sheet, r0, r1, hidden) => {
            wb.set_rows_hidden(*sheet, *r0, *r1, *hidden);
            wb.commit("hide");
        }
        Op::HideDirect(sheet, row, hidden) => {
            match hidden {
                true => wb.sheets[*sheet].hidden_rows.insert(*row),
                false => wb.sheets[*sheet].hidden_rows.remove(row),
            };
            wb.visibility_changed(*sheet);
            wb.calculate_now();
        }
        Op::DefineName(name, definition) => {
            // As typed with B1 selected: `A1` is the cell to the left.
            if let Ok(expr) = wb.parse_name_formula(definition, 0, 0, 1) {
                wb.define_name(name, expr);
                wb.commit("name");
            }
        }
        Op::DeleteName(name) => {
            wb.delete_name(NameScope::Workbook, name);
            wb.commit("name");
        }
        Op::DefineLocal(sheet, definition) => {
            if let Ok(expr) = wb.parse_name_formula(definition, 0, 0, 0) {
                let scope = NameScope::Sheet(*sheet);
                if wb
                    .edit_name(scope, "Local", "Local", expr.clone(), "")
                    .is_err()
                {
                    let _ = wb.add_name(scope, "Local", expr, "");
                }
                wb.commit("name");
            }
        }
        Op::ResizeTable(by, transaction) => {
            let Some(sheet) = wb.sheets.iter().position(|s| !s.tables.is_empty()) else {
                return;
            };
            if *transaction {
                wb.snapshot();
            }
            let range = &mut wb.sheets[sheet].tables[0].range;
            range.r1 = (range.r1 as i32 + by).clamp(range.r0 as i32 + 1, ROWS as i32 + 2) as u32;
            if *transaction {
                wb.commit("resize table");
            } else {
                // Behind the workbook's back: the next edit must notice.
                wb.set_value(sheet, ROWS + 4, 0, Value::Number(*by as f64));
                wb.commit("type");
            }
        }
        Op::AddSheet(at) => {
            wb.add_sheet(*at);
            wb.commit("sheet");
        }
        Op::DeleteSheet(sheet) => {
            wb.delete_sheet(*sheet);
            wb.commit("sheet");
        }
        Op::RenameSheet(sheet, name) => {
            wb.rename_sheet(*sheet, name);
            wb.commit("sheet");
        }
        Op::MoveSheet(from, to) => {
            wb.move_sheet(*from, *to);
            wb.commit("sheet");
        }
        Op::DuplicateSheet(sheet) => {
            wb.duplicate_sheet(*sheet);
            wb.commit("sheet");
        }
        Op::Recalc => wb.recalc(),
        Op::Manual(cells, sheet) => {
            wb.set_calc_mode(CalcMode::Manual);
            for cell in cells {
                type_cells(wb, std::slice::from_ref(cell));
                wb.commit("type");
            }
            // Now and then one of them is taken back.
            if cells.len() == 3 {
                wb.undo();
            }
            if let Some(sheet) = sheet {
                wb.calculate_sheet(*sheet);
            }
            wb.calculate_now();
            wb.set_calc_mode(CalcMode::Automatic);
        }
        Op::Iterative(enabled, iterations) => wb.set_iterative_calc(IterativeCalc {
            enabled: *enabled,
            max_iterations: *iterations,
            max_change: 0.001,
        }),
    }
}

/// A workbook of three sheets with numbers, formulas, names and a table.
pub fn build(rng: &mut Rng) -> Vec<Op> {
    let mut wb = Workbook::new(Locale::en());
    let mut ops = vec![Op::AddSheet(1), Op::AddSheet(2)];
    for op in &ops {
        apply(&mut wb, op);
    }
    let mut cells = Vec::new();
    for sheet in 0..3 {
        for row in 0..ROWS {
            for col in 0..COLS {
                if rng.chance(55) {
                    // Nothing circular to begin with.
                    let site = Site {
                        wb: &wb,
                        sheet,
                        row,
                        col,
                        wild: 0,
                    };
                    cells.push((sheet, row, col, entry(rng, &site)));
                }
            }
        }
    }
    // The table of the first sheet: headers and three rows.
    for (col, header) in ["Item", "Qty", "Price"].iter().enumerate() {
        cells.push((0, 1, col as u32 + 1, header.to_string()));
    }
    ops.push(Op::Batch(cells));
    for (name, definition) in [
        ("Rate", "Sheet1!$B$1"),
        ("Block", "Sheet1!$A$1:$B$3"),
        ("Twice", "Rate*2"),
        ("Left", "Sheet1!A1"),
    ] {
        ops.push(Op::DefineName(name.to_string(), definition.to_string()));
    }
    ops
}

/// What `build` cannot do through steps: the table and a name of sheet
/// scope.
fn furnish(wb: &mut Workbook) {
    if let Ok(id) = wb.add_table(0, Rect::new(1, 1, 4, 3), true) {
        let _ = wb.rename_table(0, id, "Sales");
    }
    wb.set_note(0, 2, 2, "fuzz", "a note");
    let local = wb
        .parse_name_formula("Sheet1!$A$1+1", 1, 0, 0)
        .expect("a formula");
    wb.add_name(NameScope::Sheet(1), "Local", local, "")
        .expect("a new name");
    wb.commit("table");
    // The steps that follow cannot undo their way out of the table.
    wb.clear_history();
}

fn describe(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("array {}x{}", a.rows, a.cols),
        other => format!("{other:?}"),
    }
}

/// The first difference between the calculated state of two workbooks
/// holding the same contents.
pub fn difference(a: &Workbook, b: &Workbook) -> Option<String> {
    if a.circular != b.circular {
        return Some(format!("circular: {} / {}", a.circular, b.circular));
    }
    if a.sheets.len() != b.sheets.len() {
        return Some("not the same sheets".to_string());
    }
    for (x, y) in a.sheets.iter().zip(&b.sheets) {
        if x.cells.len() != y.cells.len() {
            return Some(format!("{}: not the same cells", x.name));
        }
        for ((key, p), (other, q)) in x.cells.iter().zip(&y.cells) {
            let at = format!("{}!{}", x.name, cell_name(key.0, key.1));
            if key != other || p.content != q.content {
                return Some(format!("{at}: not the same content"));
            }
            if p.value != q.value {
                let formula = a.edit_text(a.sheet_index(&x.name).unwrap_or(0), key.0, key.1);
                return Some(format!(
                    "{at} ({formula}): {} / {}",
                    describe(&p.value),
                    describe(&q.value)
                ));
            }
        }
        let spills = |sheet: &opensheet_engine::Sheet| {
            let mut list: Vec<(u32, u32, usize, usize, Vec<Value>)> = sheet
                .spills
                .iter()
                .map(|s| {
                    (
                        s.row,
                        s.col,
                        s.array.rows,
                        s.array.cols,
                        s.array.data.clone(),
                    )
                })
                .collect();
            list.sort_by_key(|s| (s.0, s.1));
            list
        };
        let (p, q) = (spills(x), spills(y));
        if p != q {
            let brief = |list: &[(u32, u32, usize, usize, Vec<Value>)]| {
                list.iter()
                    .map(|s| format!("{} {}x{} {:?}", cell_name(s.0, s.1), s.2, s.3, s.4))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            return Some(format!(
                "{}: spills [{}] / [{}]",
                x.name,
                brief(&p),
                brief(&q)
            ));
        }
    }
    None
}

/// What a run went through.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    pub steps: u64,
    /// Recalculations of the incremental workbook that calculated only
    /// what changed, and those that calculated everything.
    pub partial: u64,
    pub full: u64,
    /// Steps that left a circular reference, a spilled array, an array
    /// that could not spill.
    pub circular: u64,
    pub spilling: u64,
    pub blocked: u64,
}

impl Tally {
    pub fn add(&mut self, other: &Tally) {
        self.steps += other.steps;
        self.partial += other.partial;
        self.full += other.full;
        self.circular += other.circular;
        self.spilling += other.spilling;
        self.blocked += other.blocked;
    }
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} steps, {} recalculations of what changed and {} of everything; \
             after {} steps a circular reference, after {} a spilled array, after {} a blocked one",
            self.steps, self.partial, self.full, self.circular, self.spilling, self.blocked
        )
    }
}

/// Run `steps` random steps from `seed` on two workbooks, one recalculating
/// incrementally and one in full. `Err` describes the first step after
/// which they differ, with what led there.
pub fn run(seed: u64, steps: usize) -> Result<Tally, String> {
    opensheet_engine::deps::set_audit(true);
    let mut rng = Rng::new(seed);
    let mut incremental = Workbook::new(Locale::en());
    let mut full = Workbook::new(Locale::en());
    full.set_incremental(false);
    // Two workbooks in three also calculate the precedents of a formula
    // ahead of it almost at once, as for long chains of formulas: that
    // changes nothing either.
    if seed % 3 != 0 {
        incremental.set_settle_depth(seed as u32 % 3);
    }
    let mut log: Vec<Op> = Vec::new();
    let mut tally = Tally::default();
    let check = |a: &Workbook, b: &Workbook, log: &[Op], step: usize| match difference(a, b) {
        None => Ok(()),
        Some(what) => {
            let recent: Vec<String> = log.iter().map(|op| format!("  {op:?}")).collect();
            Err(format!(
                "seed {seed}, step {step}: {what}\nsteps:\n{}",
                recent.join("\n")
            ))
        }
    };
    for op in build(&mut rng) {
        apply(&mut incremental, &op);
        apply(&mut full, &op);
        log.push(op);
    }
    furnish(&mut incremental);
    furnish(&mut full);
    check(&incremental, &full, &log, 0)?;
    for step in 1..=steps {
        let op = draw(&mut rng, &incremental);
        let before = incremental.generation;
        apply(&mut incremental, &op);
        apply(&mut full, &op);
        log.push(op);
        check(&incremental, &full, &log, step)?;
        if incremental.generation < before {
            return Err(format!(
                "seed {seed}, step {step}: the generation went back"
            ));
        }
        tally.steps += 1;
        tally.circular += incremental.circular as u64;
        tally.spilling += incremental.sheets.iter().any(|s| !s.spills.is_empty()) as u64;
        let blocked = |c: &opensheet_engine::Cell| {
            c.value == Value::Error(opensheet_engine::ErrorKind::Spill)
        };
        tally.blocked += incremental
            .sheets
            .iter()
            .any(|s| s.cells.values().any(blocked)) as u64;
    }
    let (all, partial) = incremental.calc_runs();
    tally.full = all;
    tally.partial = partial;
    // Calculating everything once more changes nothing.
    incremental.recalc();
    full.recalc();
    check(&incremental, &full, &log, steps + 1)?;
    Ok(tally)
}
