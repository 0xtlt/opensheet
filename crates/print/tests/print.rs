//! End-to-end printing tests: workbooks are printed to PDF files in a
//! temporary directory, the files are checked structurally and, when the
//! poppler tools are installed, with `pdfinfo` and `pdftotext`.
//!
//! Set `OPENSHEET_PRINT_OUT` to a directory to keep the files and look
//! at them.

use flate2::read::ZlibDecoder;
use opensheet_engine::chart::{Chart, ChartKind, ChartTitle, LegendPos};
use opensheet_engine::ops::BorderPreset;
use opensheet_engine::page::{Margins, Orientation, PageOrder, PaperSize, PrintErrors};
use opensheet_engine::refs::{Rect, parse_cell_name};
use opensheet_engine::style::{Border, BorderStyle, Color, HAlign, Style, Underline, VAlign};
use opensheet_engine::{Locale, Workbook};
use opensheet_print::{
    Canvas, FontDb, Image, ObjectPainter, Op, Paint, Path, PrintJob, PrintWhat, RectF, Stroke, TextStyle, plan,
    print_pdf, render_page,
};
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

/// 12 October 2026, 15:45.
const NOW: f64 = 46307.65625;

fn at(name: &str) -> (u32, u32) {
    parse_cell_name(name).unwrap()
}

fn enter(wb: &mut Workbook, name: &str, text: &str) {
    let (r, c) = at(name);
    wb.set_cell_text(0, r, c, text).unwrap();
}

fn style(wb: &mut Workbook, range: &str, f: impl Fn(&mut Style)) {
    wb.update_style(0, Rect::parse(range).unwrap(), &f);
}

fn job(what: PrintWhat) -> PrintJob<'static> {
    let mut job = PrintJob::new(what);
    job.now = Some(NOW);
    job.file_name = "Budget.xlsx".into();
    job.file_path = "/Users/ana/Documents/".into();
    job
}

/// Write a PDF where the tools can read it (and where the developer
/// asked to keep it).
fn save(dir: &tempfile::TempDir, name: &str, pdf: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, pdf).unwrap();
    if let Some(out) = std::env::var_os("OPENSHEET_PRINT_OUT") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(name), pdf).unwrap();
    }
    path
}

/// Output of a command-line tool; `None` when it is not installed.
fn tool(name: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(name).args(args).output().ok()?;
    assert!(
        output.status.success(),
        "{name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What poppler's `pdfinfo` says about a file.
fn pdfinfo(file: &std::path::Path, args: &[&str]) -> Option<String> {
    let file = file.to_str().unwrap();
    tool("pdfinfo", &[args, &[file]].concat())
}

/// The text of a file as poppler's `pdftotext` reads it.
fn pdftotext(file: &std::path::Path, args: &[&str]) -> Option<String> {
    let file = file.to_str().unwrap();
    tool("pdftotext", &[args, &["-layout", file, "-"]].concat())
}

/// Number of pages in the page tree, after checking that the
/// cross-reference table points at the objects.
fn page_count(pdf: &[u8]) -> usize {
    assert!(pdf.starts_with(b"%PDF-1.7\n") && pdf.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(pdf);
    let start: usize = text
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let table = String::from_utf8_lossy(&pdf[start..]).into_owned();
    let mut lines = table.lines().skip(1);
    let count: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    for (id, line) in lines.take(count).enumerate().skip(1) {
        let offset: usize = line[..10].parse().unwrap();
        assert!(
            pdf[offset..].starts_with(format!("{id} 0 obj\n").as_bytes()),
            "object {id}"
        );
    }
    let pages = text.split("/Type /Pages /Count ").nth(1).unwrap();
    pages.split(' ').next().unwrap().parse().unwrap()
}

/// The decompressed streams of a PDF (content streams, fonts, maps).
fn streams(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(pos) = find(&pdf[at..], b"/Filter /FlateDecode >>\nstream\n") {
        let start = at + pos + 31;
        let end = start + find(&pdf[start..], b"\nendstream").unwrap();
        let mut data = Vec::new();
        ZlibDecoder::new(&pdf[start..end]).read_to_end(&mut data).unwrap();
        out.push(data);
        at = end;
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A report of 121 rows and 14 columns with a header row.
fn report() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.rename_sheet(0, "Sales");
    let heads = [
        "Region", "Product", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Total", "Notes",
    ];
    for (c, head) in heads.iter().enumerate() {
        wb.set_cell_text(0, 0, c as u32, head).unwrap();
    }
    let regions = ["North", "South", "East", "West"];
    for r in 1..=120u32 {
        wb.set_cell_text(0, r, 0, regions[r as usize % 4]).unwrap();
        wb.set_cell_text(0, r, 1, &format!("Item {r:03}")).unwrap();
        for c in 2..12u32 {
            let v = ((r * 37 + c * 101) % 2000) as f64 - 400.0 + (c as f64) / 4.0;
            wb.set_cell_text(0, r, c, &format!("{v}")).unwrap();
        }
        wb.set_cell_text(0, r, 12, &format!("=SUM(C{0}:L{0})", r + 1)).unwrap();
        if r % 9 == 0 {
            wb.set_cell_text(0, r, 13, "Check with the regional office before closing")
                .unwrap();
        }
    }
    style(&mut wb, "A1:N1", |s| {
        s.font.bold = true;
        s.fill = Some(Color(0x156082));
        s.font.color = Some(Color::WHITE);
        s.h_align = HAlign::Center;
    });
    style(&mut wb, "C2:M121", |s| s.num_fmt = "#,##0.00;[Red]-#,##0.00".into());
    style(&mut wb, "M2:M121", |s| s.font.bold = true);
    wb.apply_borders(
        0,
        Rect::parse("A1:N1").unwrap(),
        BorderPreset::Bottom,
        Border {
            style: BorderStyle::Medium,
            color: None,
        },
    );
    wb.set_print_titles(0, Some((0, 0)), Some((0, 1)));
    wb.update_page_setup(0, |p| {
        p.paper = Some(PaperSize::LETTER);
        p.gridlines = true;
        p.header_footer.odd_header = "&L&A&C&\"Arial,Bold\"&14Sales report&R&D &T".into();
        p.header_footer.odd_footer = "&LConfidential&CPage &P of &N&R&Z&F".into();
    });
    wb.commit("report");
    wb
}

#[test]
fn multi_page_report() {
    let dir = tempfile::tempdir().unwrap();
    let wb = report();
    let job = job(PrintWhat::Sheet(0));
    let plan = plan(&wb, &job);
    // 14 columns do not fit across; the last one overflows onto a
    // third band because of its long notes.
    let range = &plan.sheets[0].pagination.ranges[0];
    assert_eq!(range.row_bands.len(), 3);
    assert!(range.col_bands.len() >= 2);
    assert_eq!(plan.pages.len(), plan.output.len());
    assert_eq!(plan.total as usize, plan.pages.len());
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), plan.pages.len());
    let path = save(&dir, "report.pdf", &pdf);

    if let Some(info) = pdfinfo(&path, &[]) {
        assert!(
            info.contains(&format!("Pages:           {}", plan.pages.len())),
            "{info}"
        );
        assert!(info.contains("612 x 792 pts (letter)"), "{info}");
        assert!(info.contains("Title:           Budget.xlsx"), "{info}");
    }
    if let Some(text) = pdftotext(&path, &["-f", "2", "-l", "2"]) {
        // Header, footer and the repeated title row of the second page.
        assert!(text.contains("Sales report"), "{text}");
        assert!(text.contains("10/12/2026 3:45 PM"), "{text}");
        assert!(text.contains(&format!("Page 2 of {}", plan.pages.len())), "{text}");
        assert!(text.contains("/Users/ana/Documents/Budget.xlsx"), "{text}");
        assert!(text.contains("Region") && text.contains("Product"), "{text}");
        assert!(text.contains("Item 045") || text.contains("Item 046"), "{text}");
        assert!(!text.contains("Item 001"), "{text}");
    }

    // Page range and copies.
    let mut some = job.clone();
    some.pages = Some((2, 3));
    some.copies = 2;
    assert_eq!(opensheet_print::plan(&wb, &some).output, vec![1, 2, 1, 2]);
    assert_eq!(page_count(&print_pdf(&wb, &some)), 4);
    some.collate = false;
    assert_eq!(opensheet_print::plan(&wb, &some).output, vec![1, 1, 2, 2]);
    some.pages = Some((40, 50));
    assert_eq!(page_count(&print_pdf(&wb, &some)), 0);
}

#[test]
fn landscape_fit_to_one_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut wb = Workbook::new(Locale::fr());
    for r in 0..50u32 {
        for c in 0..18u32 {
            let text = if r == 0 {
                format!("Colonne {}", c + 1)
            } else {
                format!("{}", (r * 13 + c * 7) % 500)
            };
            wb.set_cell_text(0, r, c, &text).unwrap();
        }
    }
    style(&mut wb, "A1:R1", |s| {
        s.font.bold = true;
        s.fill = Some(Color(0xDDEBF7));
    });
    wb.apply_borders(0, Rect::parse("A1:R50").unwrap(), BorderPreset::All, Border::thin());
    wb.apply_borders(
        0,
        Rect::parse("A1:R50").unwrap(),
        BorderPreset::ThickOutside,
        Border {
            style: BorderStyle::Thick,
            color: None,
        },
    );
    wb.update_page_setup(0, |p| {
        p.orientation = Orientation::Landscape;
        p.fit_to_page = true;
        p.center_horizontally = true;
        p.center_vertically = true;
        p.headings = true;
        p.header_footer.odd_footer = "&CPage &P de &N".into();
    });
    wb.commit("setup");
    let job = job(PrintWhat::Workbook);
    let plan = plan(&wb, &job);
    assert_eq!(plan.pages.len(), 1);
    let pagination = &plan.sheets[0].pagination;
    // A4 on its side; 50 rows and the heading row must fit in the
    // height: 487 pt / (51 × 15 pt) = 63 %.
    assert!((pagination.paper.0 - 841.89).abs() < 0.01 && (pagination.paper.1 - 595.28).abs() < 0.01);
    assert_eq!(pagination.scale_percent, 63);
    let page = &pagination.pages[0];
    assert!((page.origin.0 + page.size.0 / 2.0 - pagination.paper.0 / 2.0).abs() < 1e-6);
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 1);
    let path = save(&dir, "landscape.pdf", &pdf);
    if let Some(info) = pdfinfo(&path, &[]) {
        assert!(info.contains("841.89 x 595.276 pts (A4)"), "{info}");
    }
    if let Some(text) = pdftotext(&path, &[]) {
        assert!(text.contains("Colonne 18"), "{text}");
        assert!(text.contains("Page 1 de 1"), "{text}");
        // Column letters and row numbers are printed.
        assert!(
            text.lines()
                .any(|l| l.split_whitespace().collect::<Vec<_>>().starts_with(&["A", "B", "C"])),
            "{text}"
        );
    }
}

/// A sheet showing every kind of cell formatting, in French.
fn showcase() -> Workbook {
    let mut wb = Workbook::new(Locale::fr());
    wb.rename_sheet(0, "Mise en forme");
    let border = |style: BorderStyle, color: Option<Color>| Border { style, color };

    enter(&mut wb, "A1", "Démonstration de la mise en forme à l'impression");
    wb.merge(0, Rect::parse("A1:H1").unwrap());
    style(&mut wb, "A1", |s| {
        s.font.bold = true;
        s.font.size = 32;
        s.h_align = HAlign::Center;
        s.v_align = VAlign::Center;
        s.fill = Some(Color(0x0F9ED5));
        s.font.color = Some(Color::WHITE);
    });
    wb.set_row_height(0, 0, Some(36.0));

    // Borders in every line style, each cell boxed in its own style.
    let styles = [
        ("Très fin", BorderStyle::Hair),
        ("Fin", BorderStyle::Thin),
        ("Moyen", BorderStyle::Medium),
        ("Épais", BorderStyle::Thick),
        ("Tirets", BorderStyle::Dashed),
        ("Pointillés", BorderStyle::Dotted),
        ("Double", BorderStyle::Double),
    ];
    enter(&mut wb, "A3", "Bordures");
    for (i, (name, line)) in styles.iter().enumerate() {
        let row = 2 + 2 * (i as u32 / 4);
        let col = 1 + 2 * (i as u32 % 4);
        wb.set_cell_text(0, row, col, name).unwrap();
        wb.apply_borders(0, Rect::cell(row, col), BorderPreset::Outside, border(*line, None));
    }
    enter(&mut wb, "H5", "Rouge");
    wb.apply_borders(
        0,
        Rect::parse("H5").unwrap(),
        BorderPreset::Outside,
        border(BorderStyle::Medium, Some(Color(0xC00000))),
    );

    // Fills and font styles.
    enter(&mut wb, "A7", "Remplissages");
    for (i, color) in [0xFFFF00u32, 0x92D050, 0x00B0F0, 0xFFC000, 0x7030A0, 0xD9D9D9]
        .iter()
        .enumerate()
    {
        let c = *color;
        wb.set_cell_text(0, 6, 1 + i as u32, &format!("{c:06X}")).unwrap();
        wb.update_style(0, Rect::cell(6, 1 + i as u32), &move |s| {
            s.fill = Some(Color(c));
            s.h_align = HAlign::Center;
            if c == 0x7030A0 {
                s.font.color = Some(Color::WHITE);
            }
        });
    }
    enter(&mut wb, "A9", "Polices");
    enter(&mut wb, "B9", "Gras");
    style(&mut wb, "B9", |s| s.font.bold = true);
    enter(&mut wb, "C9", "Italique");
    style(&mut wb, "C9", |s| s.font.italic = true);
    enter(&mut wb, "D9", "Souligné");
    style(&mut wb, "D9", |s| s.font.underline = Underline::Single);
    enter(&mut wb, "E9", "Double");
    style(&mut wb, "E9", |s| s.font.underline = Underline::Double);
    enter(&mut wb, "F9", "Barré");
    style(&mut wb, "F9", |s| s.font.strike = true);
    enter(&mut wb, "G9", "Rouge 14");
    style(&mut wb, "G9", |s| {
        s.font.color = Some(Color(0xFF0000));
        s.font.size = 28;
    });
    enter(&mut wb, "B10", "Times New Roman");
    style(&mut wb, "B10", |s| s.font.name = "Times New Roman".into());
    enter(&mut wb, "D10", "Courier New");
    style(&mut wb, "D10", |s| s.font.name = "Courier New".into());
    enter(&mut wb, "F10", "Arial gras italique");
    style(&mut wb, "F10", |s| {
        s.font.name = "Arial".into();
        s.font.bold = true;
        s.font.italic = true;
    });

    // Alignment.
    enter(&mut wb, "A12", "Alignement");
    for (i, (text, h)) in [
        ("Gauche", HAlign::Left),
        ("Centre", HAlign::Center),
        ("Droite", HAlign::Right),
    ]
    .iter()
    .enumerate()
    {
        let h = *h;
        wb.set_cell_text(0, 11, 1 + i as u32, text).unwrap();
        wb.update_style(0, Rect::cell(11, 1 + i as u32), &move |s| s.h_align = h);
    }
    enter(&mut wb, "E12", "Retrait 2");
    style(&mut wb, "E12", |s| {
        s.h_align = HAlign::Left;
        s.indent = 2;
    });
    enter(&mut wb, "F12", "ab");
    style(&mut wb, "F12", |s| s.h_align = HAlign::Fill);
    enter(&mut wb, "G12", "Sur deux");
    style(&mut wb, "G12:H12", |s| s.h_align = HAlign::CenterAcross);
    wb.set_row_height(0, 12, Some(44.0));
    for (i, (text, v)) in [
        ("Haut", VAlign::Top),
        ("Milieu", VAlign::Center),
        ("Bas", VAlign::Bottom),
    ]
    .iter()
    .enumerate()
    {
        let v = *v;
        wb.set_cell_text(0, 12, 1 + i as u32, text).unwrap();
        wb.update_style(0, Rect::cell(12, 1 + i as u32), &move |s| s.v_align = v);
    }
    wb.apply_borders(
        0,
        Rect::parse("B12:H13").unwrap(),
        BorderPreset::All,
        border(BorderStyle::Hair, None),
    );

    // Wrapped, justified, shrunk and rotated text.
    enter(&mut wb, "A15", "Texte");
    enter(
        &mut wb,
        "B15",
        "Renvoi à la ligne automatique : l'été, les élèves français goûtent des crêpes à Noël.",
    );
    wb.merge(0, Rect::parse("B15:C15").unwrap());
    style(&mut wb, "B15", |s| {
        s.wrap = true;
        s.v_align = VAlign::Top;
    });
    enter(
        &mut wb,
        "D15",
        "Texte justifié sur toute la largeur de la cellule fusionnée, sauf la dernière ligne.",
    );
    wb.merge(0, Rect::parse("D15:E15").unwrap());
    style(&mut wb, "D15", |s| {
        s.h_align = HAlign::Justify;
        s.v_align = VAlign::Top;
    });
    enter(&mut wb, "F15", "Ajusté à la cellule");
    style(&mut wb, "F15", |s| s.shrink_to_fit = true);
    enter(&mut wb, "G15", "Pivoté 45°");
    style(&mut wb, "G15", |s| s.rotation = 45);
    enter(&mut wb, "H15", "Vertical");
    style(&mut wb, "H15", |s| {
        s.rotation = 90;
        s.h_align = HAlign::Center;
    });
    wb.set_row_height(0, 14, Some(84.0));
    wb.apply_borders(0, Rect::parse("B15:H15").unwrap(), BorderPreset::All, Border::thin());

    // Overflow, clipping and numbers.
    enter(&mut wb, "A17", "Débordement");
    enter(&mut wb, "B17", "Ce texte déborde sur les cellules vides voisines");
    enter(&mut wb, "B18", "Ce texte est coupé par la cellule voisine");
    enter(&mut wb, "C18", "voisine");
    enter(&mut wb, "H19", "Débordement vers la gauche, aligné à droite");
    style(&mut wb, "H19", |s| s.h_align = HAlign::Right);
    enter(&mut wb, "A21", "Nombres");
    enter(&mut wb, "B21", "1234567,891");
    style(&mut wb, "B21", |s| s.num_fmt = "#,##0.00 €".into());
    enter(&mut wb, "C21", "-1234,5");
    style(&mut wb, "C21", |s| s.num_fmt = "#,##0.00;[Red]-#,##0.00".into());
    enter(&mut wb, "D21", "12/10/2026");
    enter(&mut wb, "E21", "0,256");
    style(&mut wb, "E21", |s| s.num_fmt = "0.0%".into());
    enter(&mut wb, "F21", "123456789012345");
    enter(&mut wb, "G21", "1234567890123");
    style(&mut wb, "G21", |s| s.num_fmt = "#,##0.00".into());
    enter(&mut wb, "H21", "=1/0");
    enter(&mut wb, "B22", "VRAI");
    enter(&mut wb, "C22", "=C21*2");
    for col in 0..8 {
        wb.set_col_width(0, col, Some(if col == 1 { 170.0 } else { 88.0 }));
    }
    wb.commit("showcase");
    wb.update_page_setup(0, |p| {
        p.header_footer.odd_header =
            "&L&\"Times New Roman,Italic\"&12Été 2026&C&B&UEn-tête&U&B normal &KFF0000rouge&R&A".into();
        p.header_footer.odd_footer = "&LH&Y2&YO et E=mc&X2&X&C&P / &N&R&D\n&T".into();
        p.margins = Margins::NARROW;
        // One page wide whatever the fonts of the machine make of the
        // overflowing texts.
        p.fit_to_page = true;
        p.fit_height = 0;
    });
    wb.commit("page setup");
    wb
}

#[test]
fn formatting_showcase() {
    let dir = tempfile::tempdir().unwrap();
    let wb = showcase();
    let job = job(PrintWhat::Sheet(0));
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 1);
    let path = save(&dir, "showcase.pdf", &pdf);
    if let Some(text) = pdftotext(&path, &[]) {
        // Accents survive the trip through glyph numbers and back.
        for expected in [
            "Démonstration de la mise en forme à l'impression",
            "Très fin",
            "Épais",
            "Pointillés",
            "Souligné",
            "Barré",
            "Ce texte déborde sur les cellules vides voisines",
            "Été 2026",
            "Mise en forme",
            "12/10/2026",
            "15:45",
            "#DIV/0!",
            "VRAI",
            "25,6%",
        ] {
            assert!(text.contains(expected), "{expected:?} missing from:\n{text}");
        }
        // French numbers group with a narrow no-break space, printed as
        // a space; the negative number is red but reads the same.
        assert!(
            text.contains("1 234 567,89 €") || text.contains("1\u{202f}234\u{202f}567,89 €"),
            "{text}"
        );
        // The number too wide for its cell shows as hashes.
        assert!(text.contains("####"), "{text}");
        // The cut text lost its end; the neighbour is there.
        assert!(text.contains("voisine"), "{text}");
    }

    // The same sheet with the standard PDF fonts only: no font file is
    // embedded and the text is still all there.
    let fonts = FontDb::standard_only();
    let mut plain = job.clone();
    plain.fonts = Some(&fonts);
    let pdf = print_pdf(&wb, &plain);
    assert_eq!(page_count(&pdf), 1);
    let text = String::from_utf8_lossy(&pdf);
    assert!(!text.contains("/FontFile2"));
    assert!(text.contains("/BaseFont /Helvetica /Encoding /WinAnsiEncoding"));
    assert!(text.contains("/BaseFont /Times-Italic"));
    assert!(text.contains("/BaseFont /Courier"));
    let path = save(&dir, "showcase-standard.pdf", &pdf);
    if let Some(text) = pdftotext(&path, &[]) {
        assert!(
            text.contains("Démonstration de la mise en forme à l'impression"),
            "{text}"
        );
        assert!(text.contains("Été 2026") && text.contains("Pointillés"), "{text}");
    }
    // Printed twice, the bytes are the same.
    assert_eq!(pdf, print_pdf(&wb, &plain));
}

#[test]
fn drawing_matches_measuring() {
    // Every piece of text sits where its measured width says: a
    // right-aligned number ends at the cell's padding.
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "B2", "1234.5");
    enter(&mut wb, "C3", "left");
    wb.commit("cells");
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let plan = plan(&wb, &job);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    assert_eq!(scene.size, (612.0, 792.0));
    let texts: Vec<(&str, f64, f64, f64)> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Text {
                face, size, x, y, text, ..
            } => Some((text.as_str(), *x, *y, face.text_width(text, *size))),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2);
    // B2: columns of 48 pt from the 50.4 pt margin, 2.25 pt of padding.
    let (text, x, y, width) = texts[0];
    assert_eq!(text, "1234.5");
    assert!((x + width - (50.4 + 96.0 - 2.25)).abs() < 1e-6, "{x} {width}");
    // Helvetica at 11 pt: six characters, five digits and a period.
    assert!((width - (5.0 * 556.0 + 278.0) * 0.011).abs() < 1e-6);
    // The baseline: the line (1.22 × 11 pt high, the glyphs in its
    // middle) stands on the padding at the bottom of row 2.
    let line = 1.22 * 11.0;
    let ascent = (line - 0.925 * 11.0) / 2.0 + 0.718 * 11.0;
    assert!((y - (54.0 + 30.0 - 0.75 - line + ascent)).abs() < 1e-6, "{y}");
    let (text, x, _, _) = texts[1];
    assert_eq!(text, "left");
    assert!((x - (50.4 + 96.0 + 2.25)).abs() < 1e-6);
    // And the PDF positions them at the same numbers.
    let pdf = print_pdf(&wb, &job);
    let content: Vec<String> = streams(&pdf)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&s).into_owned())
        .collect();
    let page = content.iter().find(|s| s.contains(" Tm <")).unwrap();
    assert!(page.contains("1 0 0 -1 110.512 79.3505 Tm <313233342E35> Tj"), "{page}");
}

#[test]
fn print_options() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "A1", "=1/0");
    enter(&mut wb, "B1", "text");
    style(&mut wb, "B1", |s| {
        s.fill = Some(Color(0xFFFF00));
        s.font.color = Some(Color(0xFF0000));
    });
    wb.commit("cells");
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let texts = |wb: &Workbook| -> Vec<(String, Color)> {
        let plan = plan(wb, &job);
        render_page(wb, &job, &plan, 0)
            .unwrap()
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, paint, .. } => Some((text.clone(), paint.color)),
                _ => None,
            })
            .collect()
    };
    let fills = |wb: &Workbook| -> usize {
        let plan = plan(wb, &job);
        render_page(wb, &job, &plan, 0)
            .unwrap()
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Fill { .. }))
            .count()
    };
    assert_eq!(
        texts(&wb),
        vec![
            ("#DIV/0!".to_string(), Color::BLACK),
            ("text".to_string(), Color(0xFF0000))
        ]
    );
    assert_eq!(fills(&wb), 1);
    // Errors as dashes, as #N/A, or not at all.
    wb.update_page_setup(0, |p| p.errors = PrintErrors::Dash);
    assert_eq!(texts(&wb)[0].0, "--");
    wb.update_page_setup(0, |p| p.errors = PrintErrors::NA);
    assert_eq!(texts(&wb)[0].0, "#N/A");
    wb.update_page_setup(0, |p| p.errors = PrintErrors::Blank);
    assert_eq!(texts(&wb).len(), 1);
    // Black and white: no fills, black text.
    wb.update_page_setup(0, |p| p.black_and_white = true);
    assert_eq!(texts(&wb), vec![("text".to_string(), Color::BLACK)]);
    assert_eq!(fills(&wb), 0);
    // Different first page and odd / even headers.
    wb.update_page_setup(0, |p| {
        p.black_and_white = false;
        p.header_footer.odd_header = "&Codd &P".into();
        p.header_footer.even_header = "&Ceven &P".into();
        p.header_footer.first_header = "&Cfirst".into();
        p.header_footer.different_first = true;
        p.header_footer.different_odd_even = true;
        p.page_order = PageOrder::OverThenDown;
    });
    wb.set_value(0, 150, 0, opensheet_engine::Value::Number(1.0));
    wb.commit("more pages");
    let plan = plan(&wb, &job);
    assert_eq!(plan.pages.len(), 4);
    let header = |index: usize| -> String {
        let scene = render_page(&wb, &job, &plan, index).unwrap();
        scene
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, y, .. } if *y < 54.0 => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(header(0), "first");
    assert_eq!(header(1), "even 2");
    assert_eq!(header(2), "odd 3");
    assert!(render_page(&wb, &job, &plan, 9).is_none());
}

/// Draws charts as a few bars and a slice, with a caption and a picture.
struct Sketch {
    picture: Arc<Image>,
}

impl ObjectPainter for Sketch {
    fn paint_chart(&self, _wb: &Workbook, _sheet: usize, chart: &Chart, size: (f64, f64), canvas: &mut Canvas) {
        let (w, h) = size;
        canvas.fill_rect(RectF::new(0.0, 0.0, w, h), Color::WHITE);
        canvas.stroke_rect(RectF::new(0.0, 0.0, w, h), Stroke::solid(Color(0x7F7F7F), 1.0));
        let caption = TextStyle {
            family: "Arial",
            bold: true,
            italic: false,
            size: 18.0,
        };
        let title = format!("Graphique n° {}", chart.id);
        let width = canvas.text_width(&title, &caption);
        canvas.text(&title, (w - width) / 2.0, 28.0, &caption, Color(0x404040));
        for (i, value) in [0.4, 0.9, 0.6, 0.75].iter().enumerate() {
            let bar = (h - 80.0) * value;
            let x = 30.0 + i as f64 * 60.0;
            canvas.fill_rect(RectF::new(x, h - 30.0 - bar, 40.0, bar), Color(0x156082));
        }
        canvas.line(20.0, h - 30.0, 280.0, h - 30.0, Stroke::solid(Color::BLACK, 1.0));
        // A quarter of a pie, translucent over a full disc.
        let (cx, cy, r) = (w - 110.0, h / 2.0 + 10.0, 80.0);
        canvas.fill_path(
            Path::ellipse(RectF::new(cx - r, cy - r, 2.0 * r, 2.0 * r)),
            Color(0xE97132),
        );
        let slice = Path::new().move_to(cx, cy).arc(cx, cy, r, r, -90.0, 90.0).close();
        canvas.fill_path(slice.clone(), Paint::translucent(Color(0x196B24), 0.6));
        canvas.stroke_path(slice, Stroke::solid(Color::WHITE, 2.0));
        canvas.image(&self.picture, RectF::new(w - 60.0, 8.0, 48.0, 48.0));
        // Left unbalanced on purpose: the page must not be disturbed.
        canvas.save();
        canvas.translate(1000.0, 1000.0);
    }
}

#[test]
fn charts_are_painted_by_the_application() {
    let dir = tempfile::tempdir().unwrap();
    let mut wb = Workbook::new(Locale::fr());
    enter(&mut wb, "A1", "Ventes");
    enter(&mut wb, "A30", "Après le graphique");
    let chart = Chart {
        id: 0,
        kind: ChartKind::Column,
        title: ChartTitle::None,
        legend: LegendPos::None,
        series: Vec::new(),
        data_labels: false,
        anchor: (2, 1),
        offset: (8.0, 4.0),
        size: (480.0, 288.0),
        ..Chart::new(ChartKind::Column)
    };
    wb.add_chart(0, chart);
    wb.commit("chart");
    // A 16 × 16 picture: a red and blue checkerboard with a clear corner.
    let mut pixels = Vec::new();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let red = (x / 4 + y / 4) % 2 == 0;
            pixels.extend(if red { [220, 30, 30] } else { [30, 60, 200] });
            pixels.push(if x < 4 && y < 4 { 0 } else { 255 });
        }
    }
    let painter = Sketch {
        picture: Arc::new(Image::from_rgba(16, 16, pixels).unwrap()),
    };
    let mut job = job(PrintWhat::Sheet(0));
    job.painter = Some(&painter);
    let plan = plan(&wb, &job);
    assert_eq!(plan.pages.len(), 1);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    // The chart sits at B3 plus its offset: (50.4 + 72 × 0.75, 54 + 44 × 0.75).
    let moved = scene
        .ops
        .iter()
        .any(|op| matches!(op, Op::Transform(m) if (m[4] - 104.4).abs() < 1e-9 && (m[5] - 87.0).abs() < 1e-9));
    assert!(moved);
    let saves = scene.ops.iter().filter(|op| matches!(op, Op::Save)).count();
    let restores = scene.ops.iter().filter(|op| matches!(op, Op::Restore)).count();
    assert_eq!(saves, restores);
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 1);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/SMask") && text.contains("/ExtGState"));
    save(&dir, "chart.pdf", &pdf);

    // Without a painter, and in draft quality, the chart is left out but
    // still takes its place in the pagination.
    job.painter = None;
    assert_eq!(opensheet_print::plan(&wb, &job).pages.len(), 1);
    assert!(!String::from_utf8_lossy(&print_pdf(&wb, &job)).contains("/SMask"));
    job.painter = Some(&painter);
    wb.update_page_setup(0, |p| p.draft = true);
    assert!(!String::from_utf8_lossy(&print_pdf(&wb, &job)).contains("/SMask"));
}

#[test]
fn whole_workbook_and_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut wb = report();
    let second = wb.add_sheet(1);
    wb.set_cell_text(second, 0, 0, "Second sheet").unwrap();
    wb.sheets[second].page.header_footer.odd_footer = "&C&A: page &P of &N".into();
    wb.sheets[second].page.orientation = Orientation::Landscape;
    let third = wb.add_sheet(2);
    wb.commit("sheets");
    let _ = third;
    let job_all = job(PrintWhat::Workbook);
    let plan_all = plan(&wb, &job_all);
    // The empty third sheet prints nothing; numbering runs on.
    assert_eq!(plan_all.sheets.len(), 3);
    let first_pages = plan_all.sheets[0].pagination.pages.len();
    assert_eq!(plan_all.pages.len(), first_pages + 1);
    let (sheet, pagination, page) = plan_all.page(first_pages).unwrap();
    assert_eq!(sheet, second);
    assert_eq!(page.number as usize, first_pages + 1);
    assert_eq!(pagination.paper, (792.0, 612.0));
    assert_eq!(plan_all.paper(0), Some((612.0, 792.0)));
    let pdf = print_pdf(&wb, &job_all);
    assert_eq!(page_count(&pdf), first_pages + 1);
    let path = save(&dir, "workbook.pdf", &pdf);
    let last = format!("{}", first_pages + 1);
    if let Some(info) = pdfinfo(&path, &["-f", &last, "-l", &last]) {
        assert!(info.contains("792 x 612 pts"), "{info}");
    }

    // Given sheets, in the given order.
    let job_some = job(PrintWhat::Sheets(vec![second, 0, 99]));
    let plan_some = plan(&wb, &job_some);
    assert_eq!(plan_some.page(0).unwrap().0, second);
    assert_eq!(plan_some.page(1).unwrap().2.number, 2);

    // A selection prints alone, whatever the print area says.
    wb.set_print_area(0, &[Rect::parse("A1:B5").unwrap()]);
    wb.commit("print area");
    assert_eq!(plan(&wb, &job(PrintWhat::Sheet(0))).pages.len(), 1);
    let selection = PrintWhat::Selection {
        sheet: 0,
        ranges: vec![Rect::parse("C10:E12").unwrap()],
    };
    let plan_selection = plan(&wb, &job(selection.clone()));
    assert_eq!(plan_selection.pages.len(), 1);
    let page = plan_selection.page(0).unwrap().2;
    assert_eq!((page.rows, page.cols), ((9, 11), (2, 4)));
    assert_eq!(page.title_rows, Some((0, 0)));
    let mut ignore = job(PrintWhat::Sheet(0));
    ignore.ignore_print_area = true;
    assert_eq!(plan(&wb, &ignore).pages.len(), first_pages);
    save(&dir, "selection.pdf", &print_pdf(&wb, &job(selection)));

    // Nothing to print: an empty plan and a PDF without pages.
    let empty = job(PrintWhat::Sheet(third));
    assert!(plan(&wb, &empty).output.is_empty());
    assert_eq!(page_count(&print_pdf(&wb, &empty)), 0);
}

#[test]
fn embedded_fonts_are_subsets_with_text_maps() {
    // Only meaningful where a TrueType font is installed; the standard
    // fonts path is covered above.
    let fonts = FontDb::system();
    let face = fonts.face("Arial", false, false);
    if !face.is_embedded() {
        return;
    }
    let mut wb = Workbook::new(Locale::fr());
    enter(&mut wb, "A1", "Élève à Noël : 1 234,50 €");
    style(&mut wb, "A1", |s| s.font.name = "Arial".into());
    wb.commit("text");
    let job = job(PrintWhat::Sheet(0));
    let pdf = print_pdf(&wb, &job);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Subtype /CIDFontType2"));
    assert!(text.contains("/Encoding /Identity-H"));
    assert!(text.contains("/CIDToGIDMap /Identity"));
    assert!(text.contains("/FontFile2"));
    // A subset, not the better part of a megabyte the whole font takes.
    assert!(pdf.len() < 150_000, "{}", pdf.len());
    let streams = streams(&pdf);
    let map = streams
        .iter()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .find(|s| s.contains("beginbfchar"))
        .unwrap();
    // É, è, ë, € map back to text.
    for code in ["<00C9>", "<00E8>", "<00EB>", "<20AC>"] {
        assert!(map.contains(code), "{code} missing from {map}");
    }
    // The embedded font file parses and the widths written for the
    // glyphs are the font's own.
    let font_file = streams.iter().find(|s| s.starts_with(&[0, 1, 0, 0])).unwrap();
    assert!(font_file.len() > 1000);
    let width_of_e = face.text_width("e", 1000.0);
    assert!(
        text.contains(&format!("{}", (width_of_e * 10000.0).round() / 10000.0)),
        "{width_of_e}"
    );
}

/// A PNG file of one colour with a darker frame.
fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    use std::io::Write;
    let mut rows = Vec::new();
    for y in 0..height {
        rows.push(0);
        for x in 0..width {
            let edge = x == 0 || y == 0 || x == width - 1 || y == height - 1;
            rows.extend(if edge { rgb.map(|c| c / 2) } else { rgb });
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&rows).unwrap();
    let data = z.finish().unwrap();
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    for (kind, body) in [(b"IHDR", &header[..]), (b"IDAT", &data[..]), (b"IEND", &[][..])] {
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(kind);
        out.extend(body);
        out.extend([0u8; 4]);
    }
    out
}

#[test]
fn header_pictures_and_other_scripts() {
    use opensheet_engine::page::{HfPicture, HfSection, HfSlot, HfVariant};
    let dir = tempfile::tempdir().unwrap();
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "A1", "Arrows → ← and symbols Ω ≈ √ ∑");
    enter(&mut wb, "A2", "汉字 かな 한글");
    enter(&mut wb, "A3", "Rotated down");
    style(&mut wb, "A3", |s| {
        s.rotation = -60;
        s.v_align = VAlign::Top;
    });
    wb.set_row_height(0, 2, Some(90.0));
    // A merged title that the first page break cuts in two.
    enter(&mut wb, "I5", "Merged across two pages");
    wb.merge(0, Rect::parse("I5:L6").unwrap());
    style(&mut wb, "I5", |s| {
        s.h_align = HAlign::Center;
        s.v_align = VAlign::Center;
        s.fill = Some(Color(0xFFE699));
    });
    wb.apply_borders(
        0,
        Rect::parse("I5:L6").unwrap(),
        BorderPreset::Outside,
        Border {
            style: BorderStyle::Medium,
            color: None,
        },
    );
    wb.commit("cells");
    let slot = |section| HfSlot {
        variant: HfVariant::Odd,
        footer: false,
        section,
    };
    wb.update_page_setup(0, |p| {
        p.header_footer.odd_header = "&L&G&CWith a logo&R&G".into();
        p.header_footer.pictures = vec![
            HfPicture {
                slot: slot(HfSection::Left),
                data: png(40, 20, [30, 120, 200]).into(),
                size: (60.0, 30.0),
            },
            HfPicture {
                slot: slot(HfSection::Right),
                data: png(10, 10, [200, 60, 30]).into(),
                size: (24.0, 24.0),
            },
        ];
    });
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let plan = plan(&wb, &job);
    // The merged area reaches column L: two pages across.
    assert_eq!(plan.pages.len(), 2);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    let images: Vec<RectF> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    // The left picture hangs from the header margin at the left margin;
    // the right one ends at the right margin.
    assert_eq!(images.len(), 2);
    assert!((images[0].x - 50.4).abs() < 1e-9 && (images[0].y - 21.6).abs() < 1e-9);
    assert_eq!((images[0].w, images[0].h), (60.0, 30.0));
    assert!((images[1].right() - (612.0 - 50.4)).abs() < 1e-9);
    // Both pages draw the merged cell's text, each clipped to its part.
    for index in 0..2 {
        let scene = render_page(&wb, &job, &plan, index).unwrap();
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text == "Merged across two pages"))
        );
    }
    // With the standard fonts, what Windows Latin 1 lacks is a question
    // mark; the file is still sound.
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 2);
    assert_eq!(String::from_utf8_lossy(&pdf).matches("/Subtype /Image").count(), 2);

    // With the fonts of this machine, other fonts step in for the
    // characters the cell's font lacks, where there are any.
    let system = FontDb::system();
    job.fonts = None;
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 2);
    let path = save(&dir, "scripts.pdf", &pdf);
    let face = system.face("Aptos Narrow", false, false);
    let covered = |text: &str| {
        system
            .runs(&face, text)
            .iter()
            .all(|(f, run)| run.chars().all(|c| f.has_glyph(c)))
    };
    if let Some(text) = pdftotext(&path, &[]) {
        for sample in ["→", "Ω", "汉字", "かな", "한글"] {
            if covered(sample) {
                assert!(text.contains(sample), "{sample} missing from:\n{text}");
            }
        }
    }
}

/// The straight lines of a scene of a given width: (x0, y0, x1, y1).
fn lines_of(scene: &opensheet_print::Scene, width: f64) -> Vec<(f64, f64, f64, f64)> {
    use opensheet_print::Segment;
    scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Stroke { path, stroke } if (stroke.width - width).abs() < 1e-9 => match path.segments[..] {
                [Segment::MoveTo(x0, y0), Segment::LineTo(x1, y1)] => Some((x0, y0, x1, y1)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn gridlines_give_way_to_text_and_merged_cells() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "A1", "A long text that flows over its neighbours");
    enter(&mut wb, "A2", "short");
    enter(&mut wb, "B4", "merged");
    wb.merge(0, Rect::parse("B4:C5").unwrap());
    enter(&mut wb, "E6", "end");
    wb.apply_borders(
        0,
        Rect::parse("A2").unwrap(),
        BorderPreset::Bottom,
        Border {
            style: BorderStyle::Double,
            color: None,
        },
    );
    wb.update_page_setup(0, |p| p.gridlines = true);
    wb.commit("cells");
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let plan = plan(&wb, &job);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    let grid = lines_of(&scene, 0.25);
    // Columns are 48 pt wide from x = 50.4, rows 15 pt high from y = 54.
    let col = |c: f64| 50.4 + 48.0 * c;
    let row = |r: f64| 54.0 + 15.0 * r;
    let vertical_at = |x: f64, y: f64| {
        grid.iter()
            .any(|l| (l.0 - x).abs() < 1e-6 && (l.2 - x).abs() < 1e-6 && l.1 <= y && y <= l.3)
    };
    let horizontal_at = |x: f64, y: f64| {
        grid.iter()
            .any(|l| (l.1 - y).abs() < 1e-6 && (l.3 - y).abs() < 1e-6 && l.0 <= x && x <= l.2)
    };
    // The grid is closed all around A1:E6.
    assert!(vertical_at(col(0.0), row(0.5)) && vertical_at(col(5.0), row(5.5)));
    assert!(horizontal_at(col(0.5), row(0.0)) && horizontal_at(col(4.5), row(6.0)));
    assert!(!vertical_at(col(6.0), row(0.5)));
    // The text of A1 crosses the lines of the cells it flows over, on
    // its row only: it is as wide as the font says.
    let face = fonts.face("Aptos Narrow", false, false);
    let end = col(0.0) + 2.25 + face.text_width("A long text that flows over its neighbours", 11.0) + 2.25;
    assert!(end > col(3.0) && end < col(5.0));
    for c in [1.0, 2.0, 3.0, 4.0] {
        assert_eq!(
            vertical_at(col(c), row(0.5)),
            col(c) >= end,
            "line after column {c} on row 1"
        );
        assert!(vertical_at(col(c), row(1.5)), "line after column {c} on row 2");
    }
    // No lines inside the merged area, lines around it.
    assert!(!vertical_at(col(2.0), row(3.5)) && !vertical_at(col(2.0), row(4.5)));
    assert!(!horizontal_at(col(1.5), row(4.0)) && !horizontal_at(col(2.5), row(4.0)));
    assert!(horizontal_at(col(0.5), row(4.0)) && horizontal_at(col(3.5), row(4.0)));
    assert!(vertical_at(col(1.0), row(3.5)) && vertical_at(col(3.0), row(4.5)));
    assert!(horizontal_at(col(1.5), row(3.0)) && horizontal_at(col(1.5), row(5.0)));
    // A border takes the place of the gridline on its edge: the double
    // line under A2 keeps its gap.
    assert!(!horizontal_at(col(0.5), row(2.0)) && horizontal_at(col(1.5), row(2.0)));
    let double = lines_of(&scene, 0.5);
    assert_eq!(double.len(), 2);
    assert!(double.iter().all(|l| (l.0, l.2) == (col(0.0), col(1.0))));
    assert!((double[0].1 - (row(2.0) - 0.5)).abs() < 1e-9 && (double[1].1 - (row(2.0) + 0.5)).abs() < 1e-9);
    // Draft quality leaves the gridlines out.
    wb.update_page_setup(0, |p| p.draft = true);
    assert!(lines_of(&render_page(&wb, &job, &plan, 0).unwrap(), 0.25).is_empty());
}

#[test]
fn titles_repeat_and_everything_scales() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "A1", "Title");
    for r in 1..100u32 {
        wb.set_cell_text(0, r, 0, &format!("row {}", r + 1)).unwrap();
    }
    wb.apply_borders(0, Rect::parse("A1").unwrap(), BorderPreset::Bottom, Border::thin());
    wb.set_print_titles(0, Some((0, 0)), None);
    wb.update_page_setup(0, |p| {
        p.scale = 50;
        p.headings = true;
        p.header_footer.odd_header = "&C&20Big".into();
    });
    wb.commit("cells");
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let plan = plan(&wb, &job);
    assert_eq!(plan.pages.len(), 2);
    let scene = render_page(&wb, &job, &plan, 1).unwrap();
    let texts: Vec<(&str, f64, f64)> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Text { text, size, y, .. } => Some((text.as_str(), *size, *y)),
            _ => None,
        })
        .collect();
    // The title row comes first on the second page, at half size; so do
    // the headings and the header (it scales with the document).
    let title = texts.iter().find(|t| t.0 == "Title").unwrap();
    assert_eq!(title.1, 5.5);
    let first_own = plan.page(1).unwrap().2.rows.0;
    let own = texts.iter().find(|t| t.0 == format!("row {}", first_own + 1)).unwrap();
    assert!((own.2 - title.2 - 7.5).abs() < 1e-9);
    assert!(texts.iter().any(|t| t.0 == "A" && t.1 == 5.5));
    assert!(texts.iter().any(|t| t.0 == "1" && t.1 == 5.5));
    assert!(texts.iter().any(|t| t.0 == (first_own + 1).to_string()));
    assert!(texts.iter().any(|t| t.0 == "Big" && t.1 == 10.0));
    // The thin border under the title is drawn at half its width.
    assert_eq!(lines_of(&scene, 0.25).len(), 1);
    // Not scaling the header with the document.
    wb.update_page_setup(0, |p| p.header_footer.scale_with_doc = false);
    let scene = render_page(&wb, &job, &plan, 1).unwrap();
    assert!(
        scene
            .ops
            .iter()
            .any(|op| matches!(op, Op::Text { text, size, .. } if text == "Big" && *size == 20.0))
    );
}

/// The text of a scene, one string per text operation.
fn texts_of(scene: &opensheet_print::Scene) -> Vec<String> {
    scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn notes_print_at_the_end_or_where_they_are_shown() {
    use opensheet_engine::comment::Person;
    use opensheet_engine::page::PrintComments;
    let dir = tempfile::tempdir().unwrap();
    let fonts = FontDb::standard_only();
    let mut wb = Workbook::new(Locale::fr());
    enter(&mut wb, "A1", "Ventes");
    enter(&mut wb, "B2", "1200");
    enter(&mut wb, "D30", "fin");
    wb.set_note(0, 1, 1, "Ana", "Ana:\nChiffre provisoire, à confirmer avec la comptabilité avant la clôture.");
    wb.add_comment(0, 0, 0, &Person::new("Luc"), NOW, "Qui valide ?");
    wb.add_reply(0, 0, 0, &Person::new("Ana"), NOW, "Moi.");
    // A note outside the print area is left out.
    wb.set_note(0, 39, 0, "Ana", "Ana:\nHors zone");
    wb.set_print_area(0, &[Rect::parse("A1:E35").unwrap()]);
    wb.update_page_setup(0, |s| s.header_footer.odd_footer = "&CPage &P de &N".into());
    wb.commit("setup");
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);

    // Not printed unless asked for.
    let p = plan(&wb, &job);
    assert_eq!((p.pages.len(), p.sheets[0].notes.len()), (1, 0));
    let texts = texts_of(&render_page(&wb, &job, &p, 0).unwrap());
    assert!(!texts.iter().any(|t| t.contains("provisoire")));

    // At the end: one more page, numbered on, naming each cell.
    wb.update_page_setup(0, |s| s.comments = PrintComments::AtEnd);
    wb.commit("at end");
    let p = plan(&wb, &job);
    assert_eq!((p.pages.len(), p.total), (2, 2));
    assert!(p.pages[1].notes && p.page(1).is_none());
    assert_eq!(p.sheet_of(1), Some(0));
    assert_eq!(p.paper(1), p.paper(0));
    let lines: Vec<&str> = p.sheets[0].notes[0].iter().map(|line| line.text.as_str()).collect();
    assert_eq!(lines[0], "Cellule : A1");
    assert_eq!(lines[1], "Commentaire : Luc:");
    assert_eq!(&lines[2..5], &["Qui valide ?", "Ana:", "Moi."]);
    assert_eq!(&lines[5..8], &["", "Cellule : B2", "Commentaire : Ana:"]);
    assert!(lines[8].starts_with("Chiffre provisoire"));
    assert!(!lines.iter().any(|line| line.contains("Hors zone")));
    let texts = texts_of(&render_page(&wb, &job, &p, 1).unwrap());
    assert!(texts.contains(&"Cellule : B2".to_string()));
    assert!(texts.contains(&"Page 2 de 2".to_string()));
    let pdf = print_pdf(&wb, &job);
    assert_eq!(page_count(&pdf), 2);
    let path = save(&dir, "notes-at-end.pdf", &pdf);
    if let Some(text) = pdftotext(&path, &["-f", "2"]) {
        assert!(text.contains("Cellule : A1") && text.contains("Qui valide ?"));
    }
    // A long text takes as many pages as it needs.
    let long = format!("Ana:\n{}", "ligne\n".repeat(120));
    wb.set_note(0, 2, 2, "Ana", &long);
    wb.commit("long");
    let p = plan(&wb, &job);
    assert_eq!(p.sheets[0].notes.len(), 3);
    assert_eq!(p.total, 4);

    // As displayed: only the notes shown on the sheet, in their boxes.
    wb.delete_comment(0, 2, 2);
    wb.update_page_setup(0, |s| s.comments = PrintComments::AsDisplayed);
    wb.commit("as displayed");
    let p = plan(&wb, &job);
    assert_eq!(p.pages.len(), 1);
    let scene = render_page(&wb, &job, &p, 0).unwrap();
    assert!(!texts_of(&scene).iter().any(|t| t.contains("provisoire")));
    wb.set_note_visible(0, 1, 1, true);
    wb.commit("show");
    let scene = render_page(&wb, &job, &p, 0).unwrap();
    let texts = texts_of(&scene);
    assert!(texts.iter().any(|t| t == "Ana:"));
    assert!(texts.iter().any(|t| t.starts_with("Chiffre provisoire")));
    // The text is wrapped inside the box: 144 pixels are 108 points.
    let frame = wb.sheets[0].note_frame(1, 1).unwrap();
    let boxes: Vec<&RectF> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Clip(r) if (r.w - frame.width * 0.75).abs() < 1e-6 => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 1);
    for op in &scene.ops {
        if let Op::Text { face, size, x, text, .. } = op {
            if text.starts_with("Chiffre") {
                assert!(x + face.text_width(text, *size) <= boxes[0].right() + 1e-6);
            }
        }
    }
    save(&dir, "notes-as-displayed.pdf", &print_pdf(&wb, &job));
}

#[test]
fn sparklines_print_in_their_cells() {
    use opensheet_engine::sparkline::SparklineType;
    let mut wb = Workbook::new(Locale::en());
    // What prints is the block of cells that hold something: F5 brings
    // the cells of the sparklines into it.
    for (cell, text) in [("A1", "1"), ("B1", "4"), ("C1", "2"), ("A2", "3"), ("B2", "-1"), ("C2", "2"), ("F5", "x")] {
        enter(&mut wb, cell, text);
    }
    let data = Rect::parse("A1:C2").unwrap();
    wb.add_sparklines(0, SparklineType::Line, (0, Rect::parse("A1:C1").unwrap()), Rect::parse("E1").unwrap()).unwrap();
    wb.add_sparklines(0, SparklineType::Column, (0, data), Rect::parse("E3:E4").unwrap()).unwrap();
    wb.commit("sparklines");
    let fonts = FontDb::standard_only();
    let mut job = job(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    let plan = plan(&wb, &job);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    // The line: one stroke of three points in Excel's blue, inside E1
    // (columns of 48 pt from the 50.4 pt margin, rows of 15 pt from 54).
    let strokes: Vec<_> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Stroke { path, stroke } if stroke.paint.color == Color(0x376092) => Some((path, stroke.width)),
            _ => None,
        })
        .collect();
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].1, 1.0);
    // Six columns in the two cells under it.
    let bars = scene
        .ops
        .iter()
        .filter(|op| matches!(op, Op::Fill { paint, .. } if paint.color == Color(0x376092)))
        .count();
    assert_eq!(bars, 6);
    // Each cell is drawn in its own box, clipped to it.
    let clips: Vec<&RectF> = scene.ops.iter().filter_map(|op| if let Op::Clip(r) = op { Some(r) } else { None }).collect();
    let e1 = clips.iter().find(|r| (r.x - (50.4 + 4.0 * 48.0)).abs() < 1e-6 && (r.y - 54.0).abs() < 1e-6 && (r.w - 48.0).abs() < 1e-6);
    assert!(e1.is_some(), "{clips:?}");
    assert!(!print_pdf(&wb, &job).is_empty());
}

#[test]
fn the_background_of_a_sheet_is_not_printed() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "A1", "Over the background");
    let job = job(PrintWhat::Sheet(0));
    let plain = render_page(&wb, &job, &plan(&wb, &job), 0).unwrap();
    let picture = opensheet_engine::drawing::Picture::new(png(24, 24, [200, 220, 255])).expect("a PNG image");
    assert!(wb.set_sheet_background(0, Some(picture)));
    wb.commit("background");
    // The page is the same with and without it, down to the last operation.
    let plan = plan(&wb, &job);
    assert_eq!(plan.pages.len(), 1);
    let scene = render_page(&wb, &job, &plan, 0).unwrap();
    assert!(!scene.ops.iter().any(|op| matches!(op, Op::Image { .. })));
    assert_eq!(scene.ops.len(), plain.ops.len());
    assert!(!String::from_utf8_lossy(&print_pdf(&wb, &job)).contains("/Subtype /Image"));
}

#[test]
fn a_range_is_drawn_alone_without_the_furniture_of_a_page() {
    use opensheet_print::{RangeLook, render_range, scene_svg};
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, "B2", "Tea");
    enter(&mut wb, "C2", "12.5");
    enter(&mut wb, "A1", "outside");
    style(&mut wb, "B2", |s| s.fill = Some(Color(0xFF0000)));
    wb.update_page_setup(0, |setup| {
        setup.header_footer.odd_header = "&CConfidential".to_string();
        setup.headings = true;
    });
    wb.commit("setup");
    let fonts = FontDb::standard_only();
    let range = Rect::parse("B2:C3").unwrap();
    let scene = render_range(&wb, 0, range, RangeLook::Screen { gridlines: true }, Some(&fonts), None).unwrap();
    // The page is the range: three quarters of a point for each pixel.
    let sheet = &wb.sheets[0];
    let width = sheet.col_width(1) as f64 + sheet.col_width(2) as f64;
    assert!((scene.size.0 - width * 0.75).abs() < 0.01, "{:?}", scene.size);
    let texts: Vec<&str> = scene
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    // The cells of the range, and neither the header of the sheet, its
    // headings, nor its other cells.
    assert_eq!(texts, ["Tea", "12.5"]);
    let fills = |scene: &opensheet_print::Scene| {
        scene.ops.iter().filter(|op| matches!(op, Op::Fill { paint, .. } if paint.color == Color(0xFF0000))).count()
    };
    assert_eq!(fills(&scene), 1);
    // On screen the gridlines are the pale ones of the window; without
    // them, no line at all.
    let lines = |scene: &opensheet_print::Scene| -> Vec<Color> {
        scene
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Stroke { stroke, .. } => Some(stroke.paint.color),
                _ => None,
            })
            .collect()
    };
    assert!(!lines(&scene).is_empty() && lines(&scene).iter().all(|color| *color == Color(0xE1E1E1)));
    let bare = render_range(&wb, 0, range, RangeLook::Screen { gridlines: false }, Some(&fonts), None).unwrap();
    assert!(lines(&bare).is_empty());
    // As printed: what the page setup prints, here no gridlines and no
    // colour once the sheet prints in black and white.
    wb.update_page_setup(0, |setup| setup.black_and_white = true);
    wb.commit("setup");
    let printed = render_range(&wb, 0, range, RangeLook::Printed, Some(&fonts), None).unwrap();
    assert_eq!(fills(&printed), 0);
    assert!(lines(&printed).is_empty());
    // A hidden range shows nothing.
    wb.set_rows_hidden(0, 1, 2, true);
    wb.commit("hide");
    assert!(render_range(&wb, 0, range, RangeLook::Printed, Some(&fonts), None).is_none());
    // The document is as wide and high as asked.
    let svg = scene_svg(&scene, (300.0, 80.0));
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"300\" height=\"80\""));
    assert!(svg.contains(">Tea</text>"));
}

#[test]
fn the_entire_workbook_leaves_hidden_sheets_out() {
    use opensheet_engine::workbook::SheetVisibility;
    let mut wb = Workbook::new(Locale::en());
    for index in 1..3 {
        wb.add_sheet(index);
    }
    for sheet in 0..3 {
        wb.set_cell_text(sheet, 0, 0, &format!("page of sheet {}", sheet + 1)).unwrap();
    }
    wb.set_sheet_visibility(1, SheetVisibility::Hidden);
    wb.commit("setup");
    let job = job(PrintWhat::Workbook);
    let plan = plan(&wb, &job);
    assert_eq!(plan.sheets.iter().map(|s| s.sheet).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(plan.pages.len(), 2);
    // Asked for by itself, a hidden sheet still prints.
    assert_eq!(opensheet_print::plan(&wb, &self::job(PrintWhat::Sheet(1))).pages.len(), 1);
}
