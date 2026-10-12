//! Hyperlinks: the links a sheet keeps on its cells, where they lead, and
//! how they follow the cells.
//!
//! A link sits on a range and leads either outside the workbook (a web
//! page, a file, an e-mail address written as `mailto:`) or to a place in
//! it (a reference or a defined name). The text shown is the content of
//! the cell; the look of a link is formatting like any other.

use crate::ast::{Expr, StructuralEdit, same_name, write_sheet_name};
use crate::eval::Calc;
use crate::features::CellShift;
use crate::ops::shift_interval;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Content, Sheet, Workbook};

pub use crate::cell_style::{FOLLOWED_COLOR, LINK_COLOR};

/// The cell styles of links and of links that were followed.
pub const LINK_STYLE: &str = "Hyperlink";
pub const FOLLOWED_STYLE: &str = "Followed Hyperlink";

/// A link on a range of cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hyperlink {
    pub range: Rect,
    /// Where the link leads outside the workbook: a URL, `mailto:` and an
    /// address, or the path of a file. Empty for a place in this document.
    pub address: String,
    /// The place inside the target (or inside this workbook when
    /// `address` is empty): `Sheet1!A1`, a defined name, a bookmark.
    pub location: String,
    /// The ScreenTip; empty for the one Excel makes up.
    pub tooltip: String,
}

/// What a link leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Web,
    File,
    Email,
    /// A place in this document.
    Place,
}

/// What following a link does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Hand this URL to the system (browser, mail program, Finder).
    Open(String),
    /// Select this place of the workbook (a reference or a name).
    GoTo(String),
}

/// The characters a `mailto:` subject keeps as they are.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.~!*'()".contains(c)
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if is_plain(byte as char) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| text.get(i + 1..i + 3)).flatten();
        match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn has_prefix(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

impl Hyperlink {
    /// A link to a web page or a file.
    pub fn external(range: Rect, address: &str) -> Hyperlink {
        // What follows a `#` is a place inside the target.
        let (address, location) = match address.split_once('#') {
            Some((address, location)) if !has_prefix(address, "mailto:") => (address, location),
            _ => (address, ""),
        };
        Hyperlink {
            range,
            address: address.to_string(),
            location: location.to_string(),
            tooltip: String::new(),
        }
    }

    /// A link to a place in this document.
    pub fn place(range: Rect, location: &str) -> Hyperlink {
        Hyperlink {
            range,
            location: location.trim_start_matches('#').to_string(),
            ..Hyperlink::default()
        }
    }

    /// A link that writes an e-mail.
    pub fn email(range: Rect, address: &str, subject: &str) -> Hyperlink {
        let address = address.trim();
        let address = if has_prefix(address, "mailto:") { &address[7..] } else { address };
        let mut target = format!("mailto:{address}");
        if !subject.is_empty() {
            target.push_str("?subject=");
            target.push_str(&percent_encode(subject));
        }
        Hyperlink {
            range,
            address: target,
            ..Hyperlink::default()
        }
    }

    pub fn kind(&self) -> LinkKind {
        if self.address.is_empty() {
            LinkKind::Place
        } else if has_prefix(&self.address, "mailto:") {
            LinkKind::Email
        } else if ["http://", "https://", "ftp://"].iter().any(|scheme| has_prefix(&self.address, scheme)) {
            LinkKind::Web
        } else {
            LinkKind::File
        }
    }

    /// The address and the subject of an e-mail link.
    pub fn email_parts(&self) -> Option<(String, String)> {
        if self.kind() != LinkKind::Email {
            return None;
        }
        let rest = &self.address[7..];
        let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
        let subject = query
            .split('&')
            .find_map(|part| part.split_once('=').filter(|(key, _)| key.eq_ignore_ascii_case("subject")))
            .map(|(_, subject)| percent_decode(subject))
            .unwrap_or_default();
        Some((percent_decode(address), subject))
    }

    /// The whole target as one text: the address, `#` and the place.
    pub fn target(&self) -> String {
        match (self.address.is_empty(), self.location.is_empty()) {
            (true, _) => format!("#{}", self.location),
            (false, true) => self.address.clone(),
            (false, false) => format!("{}#{}", self.address, self.location),
        }
    }

    pub fn action(&self) -> LinkAction {
        match self.kind() {
            LinkKind::Place => LinkAction::GoTo(self.location.clone()),
            _ => LinkAction::Open(self.target()),
        }
    }

    /// The ScreenTip: the one given, or the target as Excel shows it.
    pub fn screen_tip(&self) -> String {
        if !self.tooltip.is_empty() {
            return self.tooltip.clone();
        }
        match self.kind() {
            LinkKind::Place => self.location.clone(),
            _ => self.target(),
        }
    }
}

/// The link Excel's AutoFormat makes of an entry: a web address, or an
/// e-mail address. `None` for anything else.
pub fn auto_link(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let schemes = ["http://", "https://", "ftp://", "mailto:", "file://"];
    if let Some(scheme) = schemes.iter().find(|scheme| has_prefix(text, scheme)) {
        return (text.len() > scheme.len()).then(|| text.to_string());
    }
    if has_prefix(text, "www.") && text.len() > 4 {
        return Some(format!("http://{text}/"));
    }
    // name@domain.tld, and nothing that reads like a formula or a path.
    let (name, domain) = text.split_once('@')?;
    let host_ok = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    let name_ok = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || "._%+-".contains(c));
    (host_ok && name_ok).then(|| format!("mailto:{text}"))
}

/// `a` without the cells of `b`, in up to four pieces.
fn minus(a: Rect, b: &Rect) -> Vec<Rect> {
    let Some(cut) = a.intersection(b) else {
        return vec![a];
    };
    let mut out = Vec::new();
    if cut.r0 > a.r0 {
        out.push(Rect::new(a.r0, a.c0, cut.r0 - 1, a.c1));
    }
    if cut.r1 < a.r1 {
        out.push(Rect::new(cut.r1 + 1, a.c0, a.r1, a.c1));
    }
    if cut.c0 > a.c0 {
        out.push(Rect::new(cut.r0, a.c0, cut.r1, cut.c0 - 1));
    }
    if cut.c1 < a.c1 {
        out.push(Rect::new(cut.r0, cut.c1 + 1, cut.r1, a.c1));
    }
    out
}

/// Take the links off the cells of `ranges`; a link reaching beyond them
/// stays on its other cells. Returns how many links were touched.
fn carve(links: &mut Vec<Hyperlink>, ranges: &[Rect]) -> usize {
    let mut touched = 0;
    for range in ranges {
        let mut kept = Vec::with_capacity(links.len());
        for link in links.drain(..) {
            if !link.range.intersects(range) {
                kept.push(link);
                continue;
            }
            touched += 1;
            for piece in minus(link.range, range) {
                kept.push(Hyperlink { range: piece, ..link.clone() });
            }
        }
        *links = kept;
    }
    touched
}

/// A place (`Sheet1!A1`, `'My sheet'!B2`) with its sheet renamed; `None`
/// when it does not name that sheet.
fn renamed_place(location: &str, old: &str, new: &str) -> Option<String> {
    let (sheet, cells) = location.rsplit_once('!')?;
    let plain = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(quoted) => quoted.replace("''", "'"),
        None => sheet.to_string(),
    };
    if !same_name(&plain, old) {
        return None;
    }
    let mut out = String::new();
    write_sheet_name(new, &mut out);
    out.push('!');
    out.push_str(cells);
    Some(out)
}

impl Workbook {
    /// The link on a cell.
    pub fn hyperlink_at(&self, sheet: usize, row: u32, col: u32) -> Option<&Hyperlink> {
        self.sheets.get(sheet)?.hyperlinks.iter().rev().find(|link| link.range.contains(row, col))
    }

    /// Put a link on its range, in place of those that were on these
    /// cells.
    pub fn set_hyperlink(&mut self, sheet: usize, link: Hyperlink) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        let links = &mut self.sheets[sheet].hyperlinks;
        carve(links, &[link.range]);
        links.push(link);
    }

    /// Take the links off these cells (their text and formats stay).
    /// Returns how many links were touched.
    pub fn remove_hyperlinks(&mut self, sheet: usize, ranges: &[Rect]) -> usize {
        let Some(sh) = self.sheets.get(sheet) else {
            return 0;
        };
        if !sh.hyperlinks.iter().any(|link| ranges.iter().any(|range| range.intersects(&link.range))) {
            return 0;
        }
        self.snapshot();
        carve(&mut self.sheets[sheet].hyperlinks, ranges)
    }

    /// Give cells the look of a link: the Hyperlink cell style (the
    /// Followed Hyperlink one once `followed`).
    pub fn format_as_link(&mut self, sheet: usize, range: Rect, followed: bool) {
        let style = if followed { FOLLOWED_STYLE } else { LINK_STYLE };
        // The style is one of Excel's: it is always there.
        let _ = self.apply_cell_style(sheet, &[range], style);
    }

    /// Whether a cell has one of the two styles `format_as_link` gives.
    pub fn looks_like_link(&self, sheet: usize, row: u32, col: u32) -> bool {
        let style = self.cell_style_at(sheet, row, col);
        same_name(style, LINK_STYLE) || same_name(style, FOLLOWED_STYLE)
    }

    /// Take the look of a link off the cells that have it: their font is
    /// the Normal one again.
    pub fn unformat_links(&mut self, sheet: usize, range: Rect) {
        let range = self.clamp_to_used(sheet, range);
        let normal = self.styles.default_style().font.clone();
        for row in range.r0..=range.r1 {
            for col in range.c0..=range.c1 {
                if self.looks_like_link(sheet, row, col) {
                    self.update_style(sheet, Rect::cell(row, col), &|style| {
                        style.font = normal.clone();
                        style.cell_style = None;
                    });
                }
            }
        }
    }

    /// Copy the links of a block with it (the links of the cells written
    /// are replaced), as a paste does.
    pub fn copy_hyperlinks(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        self.carry_hyperlinks(src_sheet, src, dest_sheet, dest_row, dest_col, false);
    }

    /// Move the links of a block with it, as cut and paste does.
    pub fn move_hyperlinks(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        self.carry_hyperlinks(src_sheet, src, dest_sheet, dest_row, dest_col, true);
    }

    fn carry_hyperlinks(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32, moved: bool) {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return;
        }
        let (d_row, d_col) = (dest_row as i64 - src.r0 as i64, dest_col as i64 - src.c0 as i64);
        let landing = Rect::new(
            dest_row,
            dest_col,
            (dest_row as u64 + src.rows() as u64 - 1).min(MAX_ROWS as u64 - 1) as u32,
            (dest_col as u64 + src.cols() as u64 - 1).min(MAX_COLS as u64 - 1) as u32,
        );
        let carried: Vec<Hyperlink> = self.sheets[src_sheet]
            .hyperlinks
            .iter()
            .filter_map(|link| {
                let part = link.range.intersection(&src)?;
                let range = Rect::new(
                    (part.r0 as i64 + d_row) as u32,
                    (part.c0 as i64 + d_col) as u32,
                    (part.r1 as i64 + d_row) as u32,
                    (part.c1 as i64 + d_col) as u32,
                )
                .intersection(&landing)?;
                Some(Hyperlink { range, ..link.clone() })
            })
            .collect();
        let overwritten = self.sheets[dest_sheet].hyperlinks.iter().any(|link| link.range.intersects(&landing));
        if carried.is_empty() && !overwritten {
            return;
        }
        self.snapshot();
        if moved {
            carve(&mut self.sheets[src_sheet].hyperlinks, &[src]);
        }
        let links = &mut self.sheets[dest_sheet].hyperlinks;
        carve(links, &[landing]);
        links.extend(carried);
    }

    /// Where the `HYPERLINK()` function of a cell's formula leads, when
    /// the formula has one.
    fn formula_link(&self, sheet: usize, row: u32, col: u32) -> Option<LinkAction> {
        let Content::Formula(formula) = &self.cell(sheet, row, col)?.content else {
            return None;
        };
        let mut target: Option<Expr> = None;
        formula.walk(&mut |e| {
            if let Expr::Func(name, args) = e {
                if target.is_none() && &**name == "HYPERLINK" {
                    target = args.first().cloned();
                }
            }
        });
        let text = match Calc::eval_once(self, sheet, row, col, &target?) {
            Value::Text(text) => text.to_string(),
            _ => return None,
        };
        match text.strip_prefix('#') {
            Some(place) => Some(LinkAction::GoTo(place.to_string())),
            None if text.is_empty() => None,
            None => Some(LinkAction::Open(text)),
        }
    }

    /// What a click on a cell follows: its link, or the one its
    /// `HYPERLINK()` formula makes.
    pub fn link_action_at(&self, sheet: usize, row: u32, col: u32) -> Option<LinkAction> {
        match self.hyperlink_at(sheet, row, col) {
            Some(link) => Some(link.action()),
            None => self.formula_link(sheet, row, col),
        }
    }

    /// The ScreenTip of the link a cell has (see `link_action_at`).
    pub fn link_tip_at(&self, sheet: usize, row: u32, col: u32) -> Option<String> {
        match self.hyperlink_at(sheet, row, col) {
            Some(link) => Some(link.screen_tip()),
            None => self.formula_link(sheet, row, col).map(|action| match action {
                LinkAction::Open(target) => target,
                LinkAction::GoTo(place) => place,
            }),
        }
    }
}

// ---- hooks (see `features.rs`) ------------------------------------------------

/// Links follow their cells when rows or columns come and go; those whose
/// cells are all deleted go with them.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.hyperlinks.is_empty() || !same_name(&sheet.name, edited_sheet) {
        return;
    }
    let (rows, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    sheet.hyperlinks.retain_mut(|link| {
        let range = &mut link.range;
        if rows {
            shift_interval(range.r0, range.r1, insert, at, count, MAX_ROWS).map(|(r0, r1)| (range.r0, range.r1) = (r0, r1))
        } else {
            shift_interval(range.c0, range.c1, insert, at, count, MAX_COLS).map(|(c0, c1)| (range.c0, range.c1) = (c0, c1))
        }
        .is_some()
    });
}

/// Links to a place on the renamed sheet name it anew.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for link in sheet.hyperlinks.iter_mut().filter(|link| link.address.is_empty()) {
        if let Some(location) = renamed_place(&link.location, old, new) {
            link.location = location;
        }
    }
}

/// The copy of a sheet keeps the links of the original (cloning gave them
/// to it), leading where they led.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}

/// The links of a deleted sheet go with it; those leading to it stay and
/// say the reference is not valid when followed, as in Excel.
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// Links name sheets, not positions among the tabs.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// Links follow the cells shifted inside a band and go with those that
/// are removed.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    if wb.sheets[sheet].hyperlinks.is_empty() {
        return;
    }
    wb.remove_hyperlinks(sheet, &[shift.removed()]);
    if let Some((from, to)) = shift.carried() {
        wb.move_hyperlinks(sheet, from, sheet, to.r0, to.c0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn ranges(w: &Workbook, sheet: usize) -> Vec<String> {
        let mut out: Vec<String> = w.sheets[sheet].hyperlinks.iter().map(|link| link.range.name()).collect();
        out.sort();
        out
    }

    #[test]
    fn the_kinds_of_links() {
        let web = Hyperlink::external(rect("A1"), "https://example.org/page#part");
        assert_eq!((web.kind(), web.address.as_str(), web.location.as_str()), (LinkKind::Web, "https://example.org/page", "part"));
        assert_eq!(web.action(), LinkAction::Open("https://example.org/page#part".into()));
        assert_eq!(Hyperlink::external(rect("A1"), "Reports/2024.xlsx").kind(), LinkKind::File);
        let place = Hyperlink::place(rect("A1"), "Sheet2!B3");
        assert_eq!(place.kind(), LinkKind::Place);
        assert_eq!(place.action(), LinkAction::GoTo("Sheet2!B3".into()));
        assert_eq!(place.screen_tip(), "Sheet2!B3");
        let mail = Hyperlink::email(rect("A1"), "ann@example.org", "Budget 2024 & co");
        assert_eq!(mail.kind(), LinkKind::Email);
        assert_eq!(mail.address, "mailto:ann@example.org?subject=Budget%202024%20%26%20co");
        assert_eq!(mail.email_parts(), Some(("ann@example.org".into(), "Budget 2024 & co".into())));
        assert_eq!(Hyperlink::email(rect("A1"), "mailto:bob@example.org", "").address, "mailto:bob@example.org");
    }

    #[test]
    fn what_autoformat_takes_for_a_link() {
        assert_eq!(auto_link("https://example.org/a?b=1").as_deref(), Some("https://example.org/a?b=1"));
        assert_eq!(auto_link("www.example.org").as_deref(), Some("http://www.example.org/"));
        assert_eq!(auto_link("ann.lee@example.org").as_deref(), Some("mailto:ann.lee@example.org"));
        assert_eq!(auto_link("mailto:ann@example.org").as_deref(), Some("mailto:ann@example.org"));
        for plain in ["see www.example.org", "http://", "a@b", "@SUM(A1)", "price@", "example.org", ""] {
            assert_eq!(auto_link(plain), None, "{plain}");
        }
    }

    #[test]
    fn links_are_set_replaced_and_removed_with_undo() {
        let mut w = wb(&[("A1", "site")]);
        w.set_hyperlink(0, Hyperlink::external(rect("A1:B2"), "https://example.org"));
        w.commit("link");
        assert_eq!(w.hyperlink_at(0, 1, 1).map(|link| link.address.as_str()), Some("https://example.org"));
        // A link set inside another takes its cells from it.
        w.set_hyperlink(0, Hyperlink::place(rect("B2"), "Sheet1!D4"));
        w.commit("link");
        assert_eq!(w.hyperlink_at(0, 1, 1).map(Hyperlink::kind), Some(LinkKind::Place));
        assert_eq!(w.hyperlink_at(0, 0, 1).map(Hyperlink::kind), Some(LinkKind::Web));
        assert_eq!(w.remove_hyperlinks(0, &[rect("A1:A9")]), 2);
        w.commit("remove");
        assert_eq!(ranges(&w, 0), ["B1", "B2"]);
        assert!(w.undo());
        assert!(w.hyperlink_at(0, 0, 0).is_some());
        assert_eq!(w.remove_hyperlinks(0, &[rect("K1")]), 0);
    }

    #[test]
    fn links_follow_rows_columns_and_sheet_names() {
        let mut w = wb(&[("A1", "x")]);
        w.add_sheet(1);
        w.set_hyperlink(0, Hyperlink::external(rect("B2"), "https://example.org"));
        w.set_hyperlink(0, Hyperlink::place(rect("C5:C6"), "Sheet2!A1"));
        w.set_hyperlink(1, Hyperlink::place(rect("A1"), "'Sheet1'!B2"));
        w.commit("links");
        w.insert_rows(0, 0, 2);
        w.insert_cols(0, 0, 1);
        w.commit("insert");
        assert_eq!(ranges(&w, 0), ["C4", "D7:D8"]);
        w.delete_rows(0, 6, 1);
        w.commit("delete");
        assert_eq!(ranges(&w, 0), ["C4", "D7"]);
        w.delete_cols(0, 2, 1);
        w.commit("delete");
        assert_eq!(ranges(&w, 0), ["C7"]);
        // Places name the sheets as they are called now.
        w.rename_sheet(1, "Data 2");
        w.rename_sheet(0, "Home");
        w.commit("rename");
        assert_eq!(w.sheets[0].hyperlinks[0].location, "'Data 2'!A1");
        assert_eq!(w.sheets[1].hyperlinks[0].location, "Home!B2");
        // A copy of the sheet has the links of the original.
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        assert_eq!(ranges(&w, copy), ["C7"]);
    }

    #[test]
    fn links_travel_with_copied_and_moved_cells() {
        let mut w = wb(&[("A1", "x"), ("D1", "old")]);
        w.set_hyperlink(0, Hyperlink::external(rect("A1"), "https://example.org"));
        w.set_hyperlink(0, Hyperlink::external(rect("D1"), "https://old.example.org"));
        w.commit("links");
        w.copy_hyperlinks(0, rect("A1:B2"), 0, 0, 3);
        w.commit("copy");
        assert_eq!(ranges(&w, 0), ["A1", "D1"]);
        assert_eq!(w.hyperlink_at(0, 0, 3).unwrap().address, "https://example.org");
        w.move_hyperlinks(0, rect("A1"), 0, 9, 0);
        w.commit("move");
        assert_eq!(ranges(&w, 0), ["A10", "D1"]);
        // Copying cells without links over a link takes it away.
        w.copy_hyperlinks(0, rect("F1"), 0, 0, 3);
        w.commit("copy");
        assert_eq!(ranges(&w, 0), ["A10"]);
    }

    #[test]
    fn the_hyperlink_function_is_followed_too() {
        let w = wb(&[
            ("A1", "example.org"),
            ("B1", "=HYPERLINK(\"https://\"&A1,\"Site\")"),
            ("B2", "=HYPERLINK(\"#Sheet1!C3\")"),
            ("B3", "=IF(TRUE,HYPERLINK(\"mailto:ann@example.org\",\"Ann\"),\"\")"),
            ("B4", "=1+1"),
        ]);
        assert_eq!(w.link_action_at(0, 0, 1), Some(LinkAction::Open("https://example.org".into())));
        assert_eq!(w.link_action_at(0, 1, 1), Some(LinkAction::GoTo("Sheet1!C3".into())));
        assert_eq!(w.link_action_at(0, 2, 1), Some(LinkAction::Open("mailto:ann@example.org".into())));
        assert_eq!(w.link_action_at(0, 3, 1), None);
        assert_eq!(w.link_tip_at(0, 0, 1).as_deref(), Some("https://example.org"));
    }

    #[test]
    fn the_look_of_a_link() {
        let mut w = wb(&[("A1", "x"), ("A2", "y")]);
        w.format_as_link(0, rect("A1"), false);
        w.update_style(0, rect("A2"), &|s| s.font.underline = crate::style::Underline::Single);
        w.commit("format");
        assert!(w.looks_like_link(0, 0, 0));
        assert_eq!(w.cell_style_at(0, 0, 0), "Hyperlink");
        assert_eq!(w.style_at(0, 0, 0).font.color, Some(LINK_COLOR));
        assert!(!w.looks_like_link(0, 1, 0));
        w.format_as_link(0, rect("A1"), true);
        assert_eq!(w.style_at(0, 0, 0).font.color, Some(FOLLOWED_COLOR));
        w.unformat_links(0, rect("A1:A2"));
        w.commit("unformat");
        assert_eq!(w.style_at(0, 0, 0), w.styles.default_style());
        // Underlining that was not a link's stays.
        assert_eq!(w.style_at(0, 1, 0).font.underline, crate::style::Underline::Single);
    }
}
