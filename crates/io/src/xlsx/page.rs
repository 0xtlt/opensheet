//! Page setup: how a sheet prints.
//!
//! It is spread over the worksheet part — `<pageSetUpPr>` inside
//! `<sheetPr>`, then `<printOptions>`, `<pageMargins>`, `<pageSetup>`,
//! `<headerFooter>`, `<rowBreaks>` and `<colBreaks>` in a row — and the
//! workbook part, where the print area and the print titles of a sheet
//! are defined names with reserved spellings (`_xlnm.Print_Area`,
//! `_xlnm.Print_Titles`) scoped to that sheet.
//!
//! The pictures of headers and footers (`<legacyDrawingHF>` and its VML
//! part) are not read or written.

use super::write::push_number;
use super::xml::{Event, Reader, Tag, decode_cell_text, push_cell_text, push_text};
use opensheet_engine::page::{
    HeaderFooter, Margins, Orientation, PageOrder, PageSetup, PaperSize, PrintComments, PrintErrors,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::{Sheet, Workbook};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The reserved names of the print area and of the print titles, as the
/// reader compares them (in upper case).
const PRINT_AREA: &str = "_XLNM.PRINT_AREA";
const PRINT_TITLES: &str = "_XLNM.PRINT_TITLES";

/// Longest header or footer Excel takes.
const MAX_HEADER_LEN: usize = 255;
/// Widest margin kept, in inches: no paper Excel knows is larger.
const MAX_MARGIN: f64 = 49.0;
/// Most pages "Fit to" can be given.
const MAX_FIT: u32 = 32_767;

// ---- reading -------------------------------------------------------------------

/// `<pageSetUpPr>` inside `<sheetPr>`.
pub(crate) fn read_sheet_pr(tag: &Tag, page: &mut PageSetup) {
    page.fit_to_page = tag.attr_bool("fitToPage").unwrap_or(false);
}

fn read_print_options(tag: &Tag, page: &mut PageSetup) {
    let flag = |name: &str| tag.attr_bool(name).unwrap_or(false);
    page.center_horizontally = flag("horizontalCentered");
    page.center_vertically = flag("verticalCentered");
    page.headings = flag("headings");
    // `gridLinesSet` off means the gridlines setting is not to be used.
    page.gridlines = flag("gridLines") && tag.attr_bool("gridLinesSet").unwrap_or(true);
}

fn read_margins(tag: &Tag, page: &mut PageSetup) {
    let inches = |name: &str, default: f64| {
        tag.attr_f64(name)
            .filter(|inches| (0.0..=MAX_MARGIN).contains(inches))
            .unwrap_or(default)
    };
    let normal = Margins::NORMAL;
    page.margins = Margins {
        left: inches("left", normal.left),
        right: inches("right", normal.right),
        top: inches("top", normal.top),
        bottom: inches("bottom", normal.bottom),
        header: inches("header", normal.header),
        footer: inches("footer", normal.footer),
    };
}

fn read_page_setup(tag: &Tag, page: &mut PageSetup) {
    // Without `paperSize` the sheet prints on the paper of the printer,
    // whatever that is where the file is opened: no paper of its own.
    page.paper = tag
        .attr_u32("paperSize")
        .filter(|code| (1..=u16::MAX as u32).contains(code))
        .map(|code| PaperSize(code as u16));
    page.orientation = Orientation::from_ooxml(&tag.attr("orientation").unwrap_or_default());
    page.scale = tag
        .attr_u32("scale")
        .filter(|scale| (10..=400).contains(scale))
        .map_or(100, |scale| scale as u16);
    let fit = |name: &str| tag.attr_u32(name).map_or(1, |n| n.min(MAX_FIT) as u16);
    page.fit_width = fit("fitToWidth");
    page.fit_height = fit("fitToHeight");
    page.first_page_number = tag
        .attr_bool("useFirstPageNumber")
        .unwrap_or(false)
        .then(|| {
            // Excel stores negative numbers as large unsigned ones.
            let number = tag
                .attr("firstPageNumber")
                .and_then(|n| n.trim().parse::<i64>().ok())
                .unwrap_or(1);
            (number as u32) as i32
        });
    page.page_order = PageOrder::from_ooxml(&tag.attr("pageOrder").unwrap_or_default());
    page.black_and_white = tag.attr_bool("blackAndWhite").unwrap_or(false);
    page.draft = tag.attr_bool("draft").unwrap_or(false);
    page.comments = PrintComments::from_ooxml(&tag.attr("cellComments").unwrap_or_default());
    page.errors = PrintErrors::from_ooxml(&tag.attr("errors").unwrap_or_default());
    // Excel writes huge numbers when the printer told it nothing.
    let dpi = |name: &str| tag.attr_u32(name).filter(|dpi| (1..=0xFFFF).contains(dpi));
    page.horizontal_dpi = dpi("horizontalDpi");
    page.vertical_dpi = dpi("verticalDpi");
}

fn read_header_footer<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, page: &mut PageSetup) {
    let hf = &mut page.header_footer;
    hf.different_odd_even = tag.attr_bool("differentOddEven").unwrap_or(false);
    hf.different_first = tag.attr_bool("differentFirst").unwrap_or(false);
    hf.scale_with_doc = tag.attr_bool("scaleWithDoc").unwrap_or(true);
    hf.align_with_margins = tag.attr_bool("alignWithMargins").unwrap_or(true);
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let text = decode_cell_text(&r.text(&t)).into_owned();
                match t.name {
                    "oddHeader" => hf.odd_header = text,
                    "oddFooter" => hf.odd_footer = text,
                    "evenHeader" => hf.even_header = text,
                    "evenFooter" => hf.even_footer = text,
                    "firstHeader" => hf.first_header = text,
                    "firstFooter" => hf.first_footer = text,
                    _ => {}
                }
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

/// The manual breaks of `<rowBreaks>` or `<colBreaks>`: the rows (or
/// columns) that start a new page. `count` is how many rows (or columns)
/// a sheet has.
fn read_breaks<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, count: u32) -> BTreeSet<u32> {
    let mut breaks = BTreeSet::new();
    if tag.empty {
        return breaks;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                // Only the breaks someone put there are stored; the
                // others follow from the paper.
                if t.name == "brk"
                    && t.attr_bool("man").unwrap_or(false)
                    && let Some(id) = t.attr_u32("id").filter(|id| (1..count).contains(id))
                {
                    breaks.insert(id);
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    breaks
}

/// Read one of the page setup elements of a worksheet. Returns false,
/// having read nothing, when `tag` is not one of them.
pub(crate) fn read<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet) -> bool {
    let page = &mut sheet.page;
    match tag.name {
        "printOptions" => read_print_options(tag, page),
        "pageMargins" => read_margins(tag, page),
        "pageSetup" => read_page_setup(tag, page),
        "headerFooter" => {
            read_header_footer(r, tag, page);
            return true;
        }
        "rowBreaks" => {
            page.row_breaks = read_breaks(r, tag, MAX_ROWS);
            return true;
        }
        "colBreaks" => {
            page.col_breaks = read_breaks(r, tag, MAX_COLS);
            return true;
        }
        _ => return false,
    }
    r.skip(tag);
    true
}

/// Take a defined name of the workbook part if it is the print area or
/// the print titles of a sheet (`name` in upper case). Returns whether it
/// was one of the two, read or not.
pub(crate) fn read_name(name: &str, text: &str, sheet: Option<&mut Sheet>) -> bool {
    if name != PRINT_AREA && name != PRINT_TITLES {
        return false;
    }
    // A print area that belongs to no sheet prints nothing.
    if let Some(sheet) = sheet {
        // What is not plain ranges (`#REF!` after the cells were deleted)
        // is no setting.
        if name == PRINT_AREA {
            sheet.page.set_print_area_formula(text);
        } else {
            sheet.page.set_print_titles_formula(text);
        }
    }
    true
}

// ---- writing -------------------------------------------------------------------

/// Append the child of `<sheetPr>` that belongs to the page setup.
pub(crate) fn push_sheet_pr(x: &mut String, page: &PageSetup) {
    if page.fit_to_page {
        x.push_str("<pageSetUpPr fitToPage=\"1\"/>");
    }
}

/// Whether `<sheetPr>` has something of the page setup to hold.
pub(crate) fn needs_sheet_pr(page: &PageSetup) -> bool {
    page.fit_to_page
}

fn push_print_options(x: &mut String, page: &PageSetup) {
    let options = [
        ("horizontalCentered", page.center_horizontally),
        ("verticalCentered", page.center_vertically),
        ("headings", page.headings),
        ("gridLines", page.gridlines),
    ];
    if options.iter().all(|(_, on)| !on) {
        return;
    }
    x.push_str("<printOptions");
    for (name, on) in options {
        if on {
            w!(x, " {name}=\"1\"");
        }
    }
    x.push_str("/>");
}

fn push_margins(x: &mut String, margins: &Margins) {
    x.push_str("<pageMargins");
    let normal = Margins::NORMAL;
    for (name, inches, default) in [
        ("left", margins.left, normal.left),
        ("right", margins.right, normal.right),
        ("top", margins.top, normal.top),
        ("bottom", margins.bottom, normal.bottom),
        ("header", margins.header, normal.header),
        ("footer", margins.footer, normal.footer),
    ] {
        let inches = if inches.is_finite() && (0.0..=MAX_MARGIN).contains(&inches) {
            inches
        } else {
            default
        };
        w!(x, " {name}=\"");
        push_number(x, inches);
        x.push('"');
    }
    x.push_str("/>");
}

fn push_page_setup(x: &mut String, page: &PageSetup) {
    let plain = PageSetup::default();
    let said = page.paper.is_some()
        || page.orientation != plain.orientation
        || page.scale != plain.scale
        || page.fit_width != plain.fit_width
        || page.fit_height != plain.fit_height
        || page.first_page_number.is_some()
        || page.page_order != plain.page_order
        || page.black_and_white
        || page.draft
        || page.comments != plain.comments
        || page.errors != plain.errors
        || page.horizontal_dpi.is_some()
        || page.vertical_dpi.is_some();
    // A sheet nobody set up says nothing, as in a file from Excel.
    if !said {
        return;
    }
    // The attributes in the order of the schema.
    x.push_str("<pageSetup");
    if let Some(paper) = page.paper.filter(|paper| paper.0 > 0) {
        w!(x, " paperSize=\"{}\"", paper.0);
    }
    if page.scale != plain.scale {
        w!(x, " scale=\"{}\"", page.scale.clamp(10, 400));
    }
    if let Some(number) = page.first_page_number {
        w!(x, " firstPageNumber=\"{}\"", number as u32);
    }
    if page.fit_width != plain.fit_width {
        w!(
            x,
            " fitToWidth=\"{}\"",
            (page.fit_width as u32).min(MAX_FIT)
        );
    }
    if page.fit_height != plain.fit_height {
        w!(
            x,
            " fitToHeight=\"{}\"",
            (page.fit_height as u32).min(MAX_FIT)
        );
    }
    if page.page_order != plain.page_order {
        w!(x, " pageOrder=\"{}\"", page.page_order.as_ooxml());
    }
    w!(x, " orientation=\"{}\"", page.orientation.as_ooxml());
    if page.black_and_white {
        x.push_str(" blackAndWhite=\"1\"");
    }
    if page.draft {
        x.push_str(" draft=\"1\"");
    }
    if page.comments != plain.comments {
        w!(x, " cellComments=\"{}\"", page.comments.as_ooxml());
    }
    if page.first_page_number.is_some() {
        x.push_str(" useFirstPageNumber=\"1\"");
    }
    if page.errors != plain.errors {
        w!(x, " errors=\"{}\"", page.errors.as_ooxml());
    }
    if let Some(dpi) = page.horizontal_dpi {
        w!(x, " horizontalDpi=\"{dpi}\"");
    }
    if let Some(dpi) = page.vertical_dpi {
        w!(x, " verticalDpi=\"{dpi}\"");
    }
    x.push_str("/>");
}

/// A header or footer cut to the length Excel takes, without leaving a
/// formatting code cut in two at the end.
fn fit_header(text: &str) -> &str {
    let mut units = 0;
    let mut end = text.len();
    for (i, c) in text.char_indices() {
        units += c.len_utf16();
        if units > MAX_HEADER_LEN {
            end = i;
            break;
        }
    }
    if end == text.len() {
        return text;
    }
    let mut cut = &text[..end];
    // A font name left open (`&"Arial,Bo`) goes whole; so does a lone `&`.
    if let Some(open) = cut.rfind("&\"")
        && !cut[open + 2..].contains('"')
    {
        cut = &cut[..open];
    }
    let ampersands = cut.len() - cut.trim_end_matches('&').len();
    if ampersands % 2 == 1 {
        cut = &cut[..cut.len() - 1];
    }
    cut
}

fn push_header_footer(x: &mut String, hf: &HeaderFooter) {
    let plain = HeaderFooter::default();
    let texts = [
        ("oddHeader", &hf.odd_header),
        ("oddFooter", &hf.odd_footer),
        ("evenHeader", &hf.even_header),
        ("evenFooter", &hf.even_footer),
        ("firstHeader", &hf.first_header),
        ("firstFooter", &hf.first_footer),
    ];
    let flags = [
        (
            "differentOddEven",
            hf.different_odd_even,
            plain.different_odd_even,
        ),
        ("differentFirst", hf.different_first, plain.different_first),
        ("scaleWithDoc", hf.scale_with_doc, plain.scale_with_doc),
        (
            "alignWithMargins",
            hf.align_with_margins,
            plain.align_with_margins,
        ),
    ];
    if texts.iter().all(|(_, text)| text.is_empty())
        && flags.iter().all(|(_, flag, default)| flag == default)
    {
        return;
    }
    x.push_str("<headerFooter");
    for (name, flag, default) in flags {
        if flag != default {
            w!(x, " {name}=\"{}\"", flag as u8);
        }
    }
    x.push('>');
    for (name, text) in texts {
        if !text.is_empty() {
            w!(x, "<{name}>");
            push_cell_text(x, fit_header(text));
            w!(x, "</{name}>");
        }
    }
    x.push_str("</headerFooter>");
}

/// `max` is the last index across: a break runs the whole way.
fn push_breaks(x: &mut String, element: &str, breaks: &BTreeSet<u32>, count: u32, max: u32) {
    let breaks: Vec<u32> = breaks
        .iter()
        .copied()
        .filter(|id| (1..count).contains(id))
        .collect();
    if breaks.is_empty() {
        return;
    }
    w!(
        x,
        "<{element} count=\"{0}\" manualBreakCount=\"{0}\">",
        breaks.len()
    );
    for id in breaks {
        w!(x, "<brk id=\"{id}\" max=\"{max}\" man=\"1\"/>");
    }
    w!(x, "</{element}>");
}

/// Append the page setup elements of a worksheet, which follow one
/// another in the schema: `printOptions`, `pageMargins`, `pageSetup`,
/// `headerFooter`, `rowBreaks`, `colBreaks`.
pub(crate) fn write(x: &mut String, page: &PageSetup) {
    push_print_options(x, page);
    push_margins(x, &page.margins);
    push_page_setup(x, page);
    push_header_footer(x, &page.header_footer);
    push_breaks(x, "rowBreaks", &page.row_breaks, MAX_ROWS, MAX_COLS - 1);
    push_breaks(x, "colBreaks", &page.col_breaks, MAX_COLS, MAX_ROWS - 1);
}

/// Append the defined names that hold the print areas and the print
/// titles of the sheets (children of `<definedNames>`).
pub(crate) fn push_names(x: &mut String, wb: &Workbook) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let names = [
            (
                "_xlnm.Print_Area",
                sheet.page.print_area_formula(&sheet.name),
            ),
            (
                "_xlnm.Print_Titles",
                sheet.page.print_titles_formula(&sheet.name),
            ),
        ];
        for (name, formula) in names {
            if let Some(formula) = formula {
                w!(x, "<definedName name=\"{name}\" localSheetId=\"{index}\">");
                push_text(x, &formula);
                x.push_str("</definedName>");
            }
        }
    }
}

/// Whether any sheet has a print area or print titles to store as names.
pub(crate) fn has_names(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sheet| {
        !sheet.page.print_area.is_empty()
            || sheet.page.title_rows.is_some()
            || sheet.page.title_cols.is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(page: &PageSetup) -> String {
        let mut x = String::new();
        write(&mut x, page);
        x
    }

    fn read_back(xml: &str) -> PageSetup {
        let mut sheet = Sheet::new("S");
        let wrapped = format!("<worksheet>{xml}</worksheet>");
        let mut r = Reader::new(&wrapped);
        loop {
            match r.next() {
                Event::Open(t) if t.name == "worksheet" => {}
                Event::Open(t) => {
                    assert!(read(&mut r, &t, &mut sheet), "{}", t.name);
                }
                Event::Eof => break,
                _ => {}
            }
        }
        sheet.page
    }

    #[test]
    fn a_sheet_nobody_set_up() {
        let page = PageSetup::default();
        assert_eq!(
            written(&page),
            "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>"
        );
        assert_eq!(read_back(&written(&page)), page);
        assert!(!needs_sheet_pr(&page));
    }

    #[test]
    fn everything_set() {
        let mut page = PageSetup {
            paper: Some(PaperSize::A4),
            orientation: Orientation::Landscape,
            scale: 85,
            fit_width: 2,
            fit_height: 0,
            first_page_number: Some(5),
            page_order: PageOrder::OverThenDown,
            horizontal_dpi: Some(600),
            vertical_dpi: Some(300),
            black_and_white: true,
            draft: true,
            comments: PrintComments::AtEnd,
            errors: PrintErrors::Dash,
            center_horizontally: true,
            center_vertically: true,
            gridlines: true,
            headings: true,
            margins: Margins {
                left: 1.0,
                right: 0.25,
                top: 1.5,
                bottom: 0.0,
                header: 0.511811023622047,
                footer: 0.3,
            },
            ..PageSetup::default()
        };
        page.row_breaks.extend([5, 40]);
        page.col_breaks.insert(3);
        page.header_footer = HeaderFooter {
            odd_header: "&L&\"Arial,Bold\"Title <1> & co&R&D".into(),
            odd_footer: "&CPage &P of &N".into(),
            even_header: "even\nsecond line".into(),
            even_footer: "&Leven footer".into(),
            first_header: "  first  ".into(),
            first_footer: "&R&F".into(),
            different_odd_even: true,
            different_first: true,
            scale_with_doc: false,
            align_with_margins: false,
            pictures: Vec::new(),
        };
        let xml = written(&page);
        assert_eq!(
            xml,
            concat!(
                "<printOptions horizontalCentered=\"1\" verticalCentered=\"1\" headings=\"1\" gridLines=\"1\"/>",
                "<pageMargins left=\"1\" right=\"0.25\" top=\"1.5\" bottom=\"0\" header=\"0.511811023622047\" footer=\"0.3\"/>",
                "<pageSetup paperSize=\"9\" scale=\"85\" firstPageNumber=\"5\" fitToWidth=\"2\" fitToHeight=\"0\" ",
                "pageOrder=\"overThenDown\" orientation=\"landscape\" blackAndWhite=\"1\" draft=\"1\" ",
                "cellComments=\"atEnd\" useFirstPageNumber=\"1\" errors=\"dash\" horizontalDpi=\"600\" verticalDpi=\"300\"/>",
                "<headerFooter differentOddEven=\"1\" differentFirst=\"1\" scaleWithDoc=\"0\" alignWithMargins=\"0\">",
                "<oddHeader>&amp;L&amp;\"Arial,Bold\"Title &lt;1&gt; &amp; co&amp;R&amp;D</oddHeader>",
                "<oddFooter>&amp;CPage &amp;P of &amp;N</oddFooter>",
                "<evenHeader>even\nsecond line</evenHeader><evenFooter>&amp;Leven footer</evenFooter>",
                "<firstHeader>  first  </firstHeader><firstFooter>&amp;R&amp;F</firstFooter></headerFooter>",
                "<rowBreaks count=\"2\" manualBreakCount=\"2\"><brk id=\"5\" max=\"16383\" man=\"1\"/>",
                "<brk id=\"40\" max=\"16383\" man=\"1\"/></rowBreaks>",
                "<colBreaks count=\"1\" manualBreakCount=\"1\"><brk id=\"3\" max=\"1048575\" man=\"1\"/></colBreaks>"
            )
        );
        assert_eq!(read_back(&xml), page);

        // A negative first page number goes the way Excel stores it.
        page.first_page_number = Some(-3);
        let xml = written(&page);
        assert!(xml.contains(" firstPageNumber=\"4294967293\" "), "{xml}");
        assert_eq!(read_back(&xml).first_page_number, Some(-3));
    }

    #[test]
    fn what_excel_writes() {
        // A sheet set to landscape on Letter with nothing else touched,
        // and one that was only given a footer.
        let page = read_back(concat!(
            "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>",
            "<pageSetup orientation=\"landscape\" horizontalDpi=\"0\" verticalDpi=\"0\" r:id=\"rId1\"/>",
            "<headerFooter><oddFooter>&amp;C&amp;P / &amp;N_x000D_</oddFooter></headerFooter>"
        ));
        assert_eq!(page.paper, None, "no paper of its own: the printer's");
        assert_eq!(page.orientation, Orientation::Landscape);
        assert_eq!((page.scale, page.fit_width, page.fit_height), (100, 1, 1));
        assert_eq!((page.horizontal_dpi, page.vertical_dpi), (None, None));
        assert_eq!(page.header_footer.odd_footer, "&C&P / &N\r");
        assert!(page.header_footer.scale_with_doc && page.header_footer.align_with_margins);
        assert_eq!(
            written(&page),
            concat!(
                "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>",
                "<pageSetup orientation=\"landscape\"/>",
                "<headerFooter><oddFooter>&amp;C&amp;P / &amp;N_x000D_</oddFooter></headerFooter>"
            )
        );
        // Letter said in so many words is a paper.
        let letter = read_back("<pageSetup paperSize=\"1\" orientation=\"portrait\"/>");
        assert_eq!(letter.paper, Some(PaperSize::LETTER));
        assert_eq!(written(&letter).matches("paperSize=\"1\"").count(), 1);
        // Automatic breaks are not stored.
        let breaks = read_back(
            "<rowBreaks count=\"3\" manualBreakCount=\"1\"><brk id=\"10\" max=\"16383\"/><brk id=\"20\" max=\"16383\" man=\"1\"/><brk id=\"30\" max=\"16383\" man=\"0\" pt=\"1\"/></rowBreaks>",
        );
        assert_eq!(breaks.row_breaks.into_iter().collect::<Vec<_>>(), [20]);
    }

    #[test]
    fn nonsense_is_ignored() {
        let page = read_back(concat!(
            "<printOptions horizontalCentered=\"maybe\" gridLines=\"1\" gridLinesSet=\"0\"/>",
            "<pageMargins left=\"-1\" right=\"1e99\" top=\"x\" bottom=\"NaN\" header=\"0.1\"/>",
            "<pageSetup paperSize=\"0\" scale=\"5000\" fitToWidth=\"99999999\" fitToHeight=\"-1\" ",
            "orientation=\"sideways\" pageOrder=\"\" cellComments=\"loud\" errors=\"none\" ",
            "useFirstPageNumber=\"1\" firstPageNumber=\"abc\" horizontalDpi=\"4294967295\"/>",
            "<headerFooter differentFirst=\"2\"><oddHeader><b>bold</b></oddHeader><weird>x</weird></headerFooter>",
            "<rowBreaks><brk id=\"0\" man=\"1\"/><brk id=\"1048576\" man=\"1\"/><brk man=\"1\"/><brk id=\"7\" man=\"1\"/><other/></rowBreaks>",
            "<colBreaks><brk id=\"16384\" man=\"1\"/><brk id=\"16383\" man=\"1\"/>"
        ));
        assert!(!page.center_horizontally && !page.gridlines);
        assert_eq!(
            page.margins,
            Margins {
                header: 0.1,
                ..Margins::NORMAL
            }
        );
        assert_eq!(page.paper, None);
        assert_eq!(
            (page.scale, page.fit_width, page.fit_height),
            (100, 32767, 1)
        );
        assert_eq!(page.orientation, Orientation::Portrait);
        assert_eq!(page.comments, PrintComments::None);
        assert_eq!(page.errors, PrintErrors::Displayed);
        assert_eq!(page.first_page_number, Some(1));
        assert_eq!(page.horizontal_dpi, None);
        assert_eq!(page.header_footer.odd_header, "bold");
        assert!(!page.header_footer.different_first);
        assert_eq!(page.row_breaks.into_iter().collect::<Vec<_>>(), [7]);
        assert_eq!(page.col_breaks.into_iter().collect::<Vec<_>>(), [16383]);
        // Values that cannot be written as they are still give a valid part.
        let mut odd = PageSetup::default();
        odd.margins.left = f64::NAN;
        odd.margins.top = -2.0;
        odd.scale = 0;
        odd.row_breaks.extend([0, 3, MAX_ROWS]);
        let xml = written(&odd);
        assert!(
            xml.contains("left=\"0.7\"") && xml.contains("top=\"0.75\""),
            "{xml}"
        );
        assert!(xml.contains(" scale=\"10\" "), "{xml}");
        assert!(xml.contains("<rowBreaks count=\"1\" manualBreakCount=\"1\"><brk id=\"3\""));
    }

    #[test]
    fn long_headers_are_cut_where_excel_stops() {
        assert_eq!(fit_header("&CPage &P"), "&CPage &P");
        let long = "x".repeat(300);
        assert_eq!(fit_header(&long).len(), 255);
        // Not in the middle of a code.
        let text = format!("{}&\"Arial,Bold\"tail", "x".repeat(250));
        assert_eq!(fit_header(&text), "x".repeat(250));
        let text = format!("{}&P and more", "x".repeat(254));
        assert_eq!(fit_header(&text), "x".repeat(254));
        // An escaped ampersand is two characters that stay together.
        let text = format!("{}&&&&", "x".repeat(253));
        assert_eq!(fit_header(&text), format!("{}&&", "x".repeat(253)));
        let emoji = "😀".repeat(200);
        assert_eq!(fit_header(&emoji).chars().count(), 127);
    }

    #[test]
    fn print_names() {
        let mut sheet = Sheet::new("Data");
        assert!(!read_name("RATE", "1", Some(&mut sheet)));
        assert!(read_name(
            PRINT_AREA,
            "Data!$A$1:$C$9,Data!$E$1:$F$9",
            Some(&mut sheet)
        ));
        assert!(read_name(
            PRINT_TITLES,
            "Data!$A:$B,Data!$1:$2",
            Some(&mut sheet)
        ));
        assert_eq!(sheet.page.print_area.len(), 2);
        assert_eq!(sheet.page.title_rows, Some((0, 1)));
        assert_eq!(sheet.page.title_cols, Some((0, 1)));
        // What is no range leaves things as they were.
        assert!(read_name(PRINT_AREA, "#REF!", Some(&mut sheet)));
        assert!(read_name(PRINT_TITLES, "Data!$A$1", Some(&mut sheet)));
        assert!(read_name(PRINT_AREA, "OFFSET(Data!$A$1,0,0,5,5)", None));
        assert_eq!(sheet.page.print_area.len(), 2);
        assert_eq!(sheet.page.title_rows, Some((0, 1)));
    }
}
