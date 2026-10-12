//! Printing for OpenSheet.
//!
//! `print_pdf` turns a workbook and a `PrintJob` into the bytes of a PDF
//! file. The work is done in three steps, each of which is also public
//! for the print preview:
//!
//! 1. `plan` paginates the sheets of the job (`opensheet_engine::page`)
//!    and numbers the pages;
//! 2. `render_page` draws one page as a `Scene`, a list of vector
//!    operations in points;
//! 3. `pdf::write_pdf` writes scenes as a PDF, embedding the fonts used.
//!
//! Charts and pictures are not drawn here: the application paints them
//! through an `ObjectPainter`, with the same vector operations.

mod fonts;
mod image;
pub mod pdf;
mod render;
mod scene;
mod std14;
mod ttf;

pub use fonts::{Face, FontDb};
pub use image::{Image, Pixels};
pub use pdf::DocInfo;
pub use scene::{Canvas, LineCap, LineJoin, Op, Paint, Path, RectF, Scene, Segment, Stroke, TextStyle};

use opensheet_engine::chart::Chart;
use opensheet_engine::condfmt::Adornment;
use opensheet_engine::datetime;
use opensheet_engine::drawing::Drawing;
use opensheet_engine::page::{HfContext, Page, PaginateOptions, Pagination, paginate};
use opensheet_engine::refs::Rect;
use opensheet_engine::workbook::Workbook;

/// Paints what this crate cannot draw by itself: charts, pictures and
/// shapes, and what conditional formats draw inside cells.
///
/// Every method receives a canvas already placed on the object: its
/// origin is the object's top-left corner, its unit is the sheet pixel
/// at 100 % (so the code that draws the object on screen can be reused
/// with the same numbers, font sizes included), and drawing is clipped to
/// the part of the page the object may cover. `size` is the object's
/// width and height in those pixels. Whatever the painter leaves saved on
/// the canvas is restored for it.
pub trait ObjectPainter {
    /// Paint a chart of the sheet.
    fn paint_chart(&self, wb: &Workbook, sheet: usize, chart: &Chart, size: (f64, f64), canvas: &mut Canvas);

    /// Paint a picture or a shape. Does nothing unless overridden.
    fn paint_drawing(&self, wb: &Workbook, sheet: usize, drawing: &Drawing, size: (f64, f64), canvas: &mut Canvas) {
        let _ = (wb, sheet, drawing, size, canvas);
    }

    /// Paint the data bar or icon a conditional format puts in the cell
    /// at `cell` (row, column), over its fill and under its text. Does
    /// nothing unless overridden.
    fn paint_adornment(
        &self,
        wb: &Workbook,
        sheet: usize,
        cell: (u32, u32),
        adornment: &Adornment,
        size: (f64, f64),
        canvas: &mut Canvas,
    ) {
        let _ = (wb, sheet, cell, adornment, size, canvas);
    }
}

/// What a job prints.
#[derive(Clone, Debug, PartialEq)]
pub enum PrintWhat {
    /// One sheet ("Print Active Sheets" with one sheet selected).
    Sheet(usize),
    /// Several sheets, in this order.
    Sheets(Vec<usize>),
    /// Every sheet ("Print Entire Workbook").
    Workbook,
    /// Ranges of one sheet ("Print Selection"), instead of its print
    /// area.
    Selection { sheet: usize, ranges: Vec<Rect> },
}

/// A request to print.
#[derive(Clone)]
pub struct PrintJob<'a> {
    pub what: PrintWhat,
    /// Print only the pages from the first to the second of these
    /// positions in the job, counted from 1; `None` prints them all.
    pub pages: Option<(u32, u32)>,
    pub copies: u32,
    /// With several copies: whole copies one after the other (1 2 3,
    /// 1 2 3) rather than each page in turn (1 1, 2 2, 3 3).
    pub collate: bool,
    /// "Ignore Print Area".
    pub ignore_print_area: bool,
    /// The workbook's file name, for `&F` and the PDF's title ("Book1"
    /// before the first save).
    pub file_name: String,
    /// The folder of the file with its trailing separator, for `&Z`.
    pub file_path: String,
    /// Recorded as the author of the PDF.
    pub author: String,
    /// The moment of printing as a date serial number, for `&D`, `&T`
    /// and the PDF's date; `None` = now.
    pub now: Option<f64>,
    /// The fonts to print with; `None` = the fonts of this machine.
    pub fonts: Option<&'a FontDb>,
    /// Draws charts and pictures; without it they are left out.
    pub painter: Option<&'a dyn ObjectPainter>,
}

impl PrintJob<'_> {
    pub fn new(what: PrintWhat) -> PrintJob<'static> {
        PrintJob {
            what,
            pages: None,
            copies: 1,
            collate: true,
            ignore_print_area: false,
            file_name: String::new(),
            file_path: String::new(),
            author: String::new(),
            now: None,
            fonts: None,
            painter: None,
        }
    }

    fn font_db(&self) -> &FontDb {
        self.fonts.unwrap_or_else(|| FontDb::system())
    }
}

/// One page of a job.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedPage {
    /// Index into `PrintPlan::sheets`.
    pub part: usize,
    /// Index into that part's `pagination.pages`.
    pub page: usize,
}

/// One sheet of a job, paginated.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedSheet {
    pub sheet: usize,
    pub pagination: Pagination,
}

/// The pages a job prints.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintPlan {
    pub sheets: Vec<PlannedSheet>,
    /// Every page of the job, in order: what a preview shows.
    pub pages: Vec<PlannedPage>,
    /// What `&N` prints: the number of the job's last page.
    pub total: i32,
    /// The pages that go to the printer, as indices into `pages`: the
    /// page range, repeated for the copies.
    pub output: Vec<usize>,
}

impl PrintPlan {
    /// The sheet index, pagination and page of a page of the job.
    pub fn page(&self, index: usize) -> Option<(usize, &Pagination, &Page)> {
        let planned = self.pages.get(index)?;
        let part = self.sheets.get(planned.part)?;
        Some((part.sheet, &part.pagination, part.pagination.pages.get(planned.page)?))
    }

    /// Paper width and height of a page in points.
    pub fn paper(&self, index: usize) -> Option<(f64, f64)> {
        self.page(index).map(|(_, pagination, _)| pagination.paper)
    }
}

/// Paginate the sheets of a job. Pages are numbered through the job: a
/// sheet whose first page number is automatic continues after the sheet
/// before it.
pub fn plan(wb: &Workbook, job: &PrintJob) -> PrintPlan {
    let (sheets, selection): (Vec<usize>, Option<&[Rect]>) = match &job.what {
        PrintWhat::Sheet(sheet) => (vec![*sheet], None),
        PrintWhat::Sheets(sheets) => (sheets.clone(), None),
        PrintWhat::Workbook => ((0..wb.sheets.len()).collect(), None),
        PrintWhat::Selection { sheet, ranges } => (vec![*sheet], Some(ranges)),
    };
    let mut out = PrintPlan {
        sheets: Vec::new(),
        pages: Vec::new(),
        total: 0,
        output: Vec::new(),
    };
    let mut next_number = None;
    for sheet in sheets.into_iter().filter(|s| *s < wb.sheets.len()) {
        let pagination = paginate(
            wb,
            sheet,
            &PaginateOptions {
                selection,
                ignore_print_area: job.ignore_print_area,
                first_page_number: next_number,
                measure: Some(job.font_db()),
            },
        );
        if let Some(last) = pagination.last_number() {
            next_number = Some(last + 1);
        }
        let part = out.sheets.len();
        out.pages.extend((0..pagination.pages.len()).map(|page| PlannedPage { part, page }));
        out.sheets.push(PlannedSheet { sheet, pagination });
    }
    // With a first page number of 5, three pages read "5 of 7".
    let first_number = out.page(0).map_or(1, |(_, _, page)| page.number);
    out.total = first_number - 1 + out.pages.len() as i32;

    let count = out.pages.len();
    let (from, to) = job.pages.unwrap_or((1, u32::MAX));
    let range: Vec<usize> = (from.max(1) as usize - 1..(to as usize).min(count)).collect();
    let copies = job.copies.max(1) as usize;
    out.output = if job.collate {
        (0..copies).flat_map(|_| range.iter().copied()).collect()
    } else {
        range.iter().flat_map(|page| std::iter::repeat_n(*page, copies)).collect()
    };
    out
}

/// Draw a page of a job (an index into `PrintPlan::pages`). `None` when
/// the plan has no such page.
pub fn render_page(wb: &Workbook, job: &PrintJob, plan: &PrintPlan, index: usize) -> Option<Scene> {
    let planned = plan.pages.get(index)?;
    let (sheet, pagination, page) = plan.page(index)?;
    let page_job = render::PageJob {
        wb,
        sheet,
        pagination,
        page,
        fonts: job.font_db(),
        painter: job.painter,
        hf: HfContext {
            page: page.number,
            pages: plan.total.max(0) as u32,
            now: job.now.unwrap_or_else(datetime::now_serial),
            file_name: &job.file_name,
            file_path: &job.file_path,
            sheet_name: &wb.sheets.get(sheet)?.name,
            locale: wb.locale,
        },
        first_of_sheet: planned.page == 0,
    };
    Some(render::render(&page_job))
}

/// Print a job as a PDF file. A job with nothing to print gives a PDF
/// without pages: check `plan(...).output` first to tell the user, as
/// Excel does.
pub fn print_pdf(wb: &Workbook, job: &PrintJob) -> Vec<u8> {
    let plan = plan(wb, job);
    // Each page is drawn once, however many copies are printed.
    let mut distinct: Vec<usize> = plan.output.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let scenes: Vec<Scene> = distinct.iter().filter_map(|index| render_page(wb, job, &plan, *index)).collect();
    let order: Vec<usize> = plan.output.iter().filter_map(|index| distinct.binary_search(index).ok()).collect();
    let info = DocInfo {
        title: job.file_name.clone(),
        author: job.author.clone(),
        created: Some(job.now.unwrap_or_else(datetime::now_serial)),
    };
    pdf::write_pdf(&scenes, &order, &info)
}
