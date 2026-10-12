//! The Page Setup dialog (Page, Margins, Header/Footer, Sheet) and the
//! dialogs it opens: Custom Header / Custom Footer and the font of their
//! text.
//!
//! The dialog works on a copy of the sheet's page setup; OK stores it as
//! one undo step.

use super::{PageUi, from_unit, metric, print, show_breaks, to_unit, views};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, Checkbox, ComboBox, GroupBox, NumberField, RadioGroup, RefEdit, Select, Sketch, TabStrip, TextField,
    dialog, icon, label, labeled, ok_cancel, preview, tooltip,
};
use gpui::{
    AnyElement, App, Context, Entity, Focusable, PathPromptOptions, Render, SharedString, WeakEntity, Window, div,
    prelude::*, px,
};
use opensheet_engine::datetime;
use opensheet_engine::drawing::image_info;
use opensheet_engine::page::{
    HeaderFooter, HfContext, HfPicture, HfRun, HfSection, HfSections, HfSlot, HfVariant, Margins, Orientation,
    PageOrder, PageSetup, PaperSize, PrintComments, PrintErrors, expand, from_dialog_text, plain_text, presets,
    to_dialog_text,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::{Locale, Rect};
use opensheet_print::FontDb;
use std::sync::Arc;

pub(crate) const TAB_PAGE: usize = 0;
pub(crate) const TAB_MARGINS: usize = 1;
pub(crate) const TAB_HEADER_FOOTER: usize = 2;
pub(crate) const TAB_SHEET: usize = 3;

/// What the dialog shows of the workbook besides the page setup.
struct Around {
    sheet: usize,
    sheet_name: String,
    /// File name and folder, for the previews of `&F` and `&Z`.
    names: (String, String),
    /// Pages the sheet prints on, for `&N`.
    pages: u32,
    locale: &'static Locale,
}

/// Open the dialog on one of its tabs.
pub(crate) fn open(app: &mut ExcelApp, tab: usize, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    app.ext_mut::<PageUi>().setup_tab = tab;
    let around = Around {
        sheet: app.sheet,
        sheet_name: app.wb.sheets[app.sheet].name.clone(),
        names: views::file_names(app),
        pages: views::pagination(app).map_or(1, |p| p.pages.len().max(1) as u32),
        locale: app.wb.locale,
    };
    let setup = app.wb.sheets[app.sheet].page.clone();
    app.open_modal(window, cx, move |modal, _, cx| {
        SetupDialog::new(modal, tab, setup, around, cx)
    });
}

// ---------------------------------------------------- what the user types

/// The ranges of "Print area", as the field shows them.
pub(crate) fn area_text(ranges: &[Rect], locale: &Locale) -> String {
    let separator = locale.arg_sep.to_string();
    ranges
        .iter()
        .map(|rect| RefStyle::absolute().format(*rect, ""))
        .collect::<Vec<_>>()
        .join(&separator)
}

/// Read "Print area": ranges separated by the list separator of the
/// locale (a comma works everywhere). `None` when a part is not a range.
pub(crate) fn parse_area(text: &str, locale: &Locale) -> Option<Vec<Rect>> {
    text.split([',', locale.arg_sep])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            // A sheet name before the range is accepted and ignored.
            let range = part.rsplit('!').next().unwrap_or(part).trim_start_matches('=');
            Rect::parse(range)
        })
        .collect()
}

/// Read "Rows to repeat at top" (`rows`) or "Columns to repeat at left":
/// whole rows or columns, or any range, of which the rows or columns are
/// taken as Excel does when cells are picked. `Err` when it is not a
/// reference.
pub(crate) fn parse_titles(text: &str, rows: bool) -> Result<Option<(u32, u32)>, ()> {
    let text = text.trim().trim_start_matches('=');
    if text.is_empty() {
        return Ok(None);
    }
    let range = text.rsplit('!').next().unwrap_or(text);
    let rect = Rect::parse(range).ok_or(())?;
    // `$1:$3` is rows, `$A:$B` columns: the other kind is a mistake.
    if rows && rect.is_whole_cols() && !rect.is_whole_rows() || !rows && rect.is_whole_rows() && !rect.is_whole_cols() {
        return Err(());
    }
    Ok(Some(if rows { (rect.r0, rect.r1) } else { (rect.c0, rect.c1) }))
}

pub(crate) fn titles_text(titles: Option<(u32, u32)>, rows: bool) -> String {
    match titles {
        None => String::new(),
        Some((first, last)) => {
            let rect = if rows {
                Rect::new(first, 0, last, MAX_COLS - 1)
            } else {
                Rect::new(0, first, MAX_ROWS - 1, last)
            };
            RefStyle::absolute().format(rect, "")
        }
    }
}

/// The first page number as the field shows it: a number or "Auto".
fn first_page_text(number: Option<i32>) -> String {
    number.map_or_else(|| t("Auto", "Auto").to_string(), |n| n.to_string())
}

/// `Ok(None)` for "Auto" or nothing, `Err` for anything that is not a
/// whole number.
pub(crate) fn parse_first_page(text: &str) -> Result<Option<i32>, ()> {
    let text = text.trim();
    if text.is_empty() || text.to_lowercase().starts_with("auto") {
        return Ok(None);
    }
    text.parse::<i32>().map(Some).map_err(|_| ())
}

/// The dots per inch offered by "Print quality".
const QUALITIES: [u32; 4] = [150, 300, 600, 1200];

// ------------------------------------------------------------- the dialog

struct SetupDialog {
    modal: Modal,
    tab: usize,
    around: Around,
    /// The working copy: what the check boxes, lists and radio buttons
    /// change directly. The text fields are read on OK.
    setup: PageSetup,
    scale: Entity<TextInput>,
    fit_width: Entity<TextInput>,
    fit_height: Entity<TextInput>,
    first_page: Entity<TextInput>,
    /// Top, bottom, left, right, header, footer, in the unit of the locale.
    margins: [Entity<TextInput>; 6],
    print_area: Entity<TextInput>,
    title_rows: Entity<TextInput>,
    title_cols: Entity<TextInput>,
}

impl SetupDialog {
    fn new(modal: Modal, tab: usize, setup: PageSetup, around: Around, cx: &mut Context<Self>) -> Self {
        let locale = around.locale;
        let m = setup.margins;
        let margin = |cx: &mut Context<Self>, inches: f64| {
            let shown = (to_unit(inches, locale) * 100.0).round() / 100.0;
            widgets::number_input(cx, Some(shown), 0.0, 500.0, if metric(locale) { 0.5 } else { 0.25 })
        };
        let pages = |cx: &mut Context<Self>, count: u16| {
            widgets::number_input(cx, (count > 0).then_some(count as f64), 0.0, 32767.0, 1.0)
        };
        SetupDialog {
            tab,
            scale: widgets::number_input(cx, Some(setup.scale as f64), 10.0, 400.0, 5.0),
            fit_width: pages(cx, setup.fit_width),
            fit_height: pages(cx, setup.fit_height),
            first_page: widgets::input(cx, first_page_text(setup.first_page_number)),
            margins: [
                margin(cx, m.top),
                margin(cx, m.bottom),
                margin(cx, m.left),
                margin(cx, m.right),
                margin(cx, m.header),
                margin(cx, m.footer),
            ],
            print_area: modal.ref_input(RefStyle::absolute(), area_text(&setup.print_area, locale), cx),
            title_rows: modal.ref_input(RefStyle::absolute(), titles_text(setup.title_rows, true), cx),
            title_cols: modal.ref_input(RefStyle::absolute(), titles_text(setup.title_cols, false), cx),
            modal,
            around,
            setup,
        }
    }

    fn context(&self) -> HfContext<'_> {
        HfContext {
            page: 1,
            pages: self.around.pages,
            now: datetime::now_serial(),
            file_name: &self.around.names.0,
            file_path: &self.around.names.1,
            sheet_name: &self.around.sheet_name,
            locale: self.around.locale,
        }
    }

    /// The margins typed in the fields, in inches.
    fn typed_margins(&self, cx: &App) -> Option<Margins> {
        let locale = self.around.locale;
        let read = |i: usize| {
            self.margins[i]
                .read(cx)
                .number()
                .filter(|v| *v >= 0.0)
                .map(|v| from_unit(v, locale))
        };
        Some(Margins {
            top: read(0)?,
            bottom: read(1)?,
            left: read(2)?,
            right: read(3)?,
            header: read(4)?,
            footer: read(5)?,
        })
    }

    fn refuse(&self, message: &'static str, cx: &mut Context<Self>) {
        self.modal
            .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
    }

    /// What the dialog says the page setup is; `None` (after telling
    /// why) when a field cannot be read.
    fn collect(&mut self, cx: &mut Context<Self>) -> Option<PageSetup> {
        let locale = self.around.locale;
        let invalid = t("Integer is not valid.", "Valeur incorrecte.");
        let mut setup = self.setup.clone();
        let whole = |field: &Entity<TextInput>, min: f64, max: f64, cx: &App| -> Result<Option<u16>, ()> {
            let field = field.read(cx);
            if field.text().trim().is_empty() {
                return Ok(None);
            }
            match field.number() {
                Some(n) if n >= min && n <= max && n.fract() == 0.0 => Ok(Some(n as u16)),
                _ => Err(()),
            }
        };
        match whole(&self.scale, 10.0, 400.0, cx) {
            Ok(Some(scale)) => setup.scale = scale,
            _ => {
                self.tab = TAB_PAGE;
                self.refuse(invalid, cx);
                return None;
            }
        }
        // A blank count is "as many pages as needed".
        match (
            whole(&self.fit_width, 0.0, 32767.0, cx),
            whole(&self.fit_height, 0.0, 32767.0, cx),
        ) {
            (Ok(width), Ok(height)) => {
                setup.fit_width = width.unwrap_or(0);
                setup.fit_height = height.unwrap_or(0);
            }
            _ => {
                self.tab = TAB_PAGE;
                self.refuse(invalid, cx);
                return None;
            }
        }
        match parse_first_page(self.first_page.read(cx).text()) {
            Ok(number) => setup.first_page_number = number,
            Err(()) => {
                self.tab = TAB_PAGE;
                self.refuse(invalid, cx);
                return None;
            }
        }
        let paper = setup.paper_pt(locale);
        match self.typed_margins(cx) {
            Some(m) if (m.left + m.right) * 72.0 < paper.0 && (m.top + m.bottom) * 72.0 < paper.1 => {
                // Fields show two decimals: margins nobody retyped keep
                // their exact value.
                let near = |typed: f64, old: f64| {
                    if (to_unit(typed, locale) - to_unit(old, locale)).abs() < 0.006 {
                        old
                    } else {
                        typed
                    }
                };
                let old = setup.margins;
                setup.margins = Margins {
                    top: near(m.top, old.top),
                    bottom: near(m.bottom, old.bottom),
                    left: near(m.left, old.left),
                    right: near(m.right, old.right),
                    header: near(m.header, old.header),
                    footer: near(m.footer, old.footer),
                };
            }
            _ => {
                self.tab = TAB_MARGINS;
                self.refuse(
                    t(
                        "Margins do not fit page size.",
                        "Marges non valides pour ce format de page.",
                    ),
                    cx,
                );
                return None;
            }
        }
        match parse_area(self.print_area.read(cx).text(), locale) {
            Some(ranges) => setup.print_area = ranges,
            None => {
                self.tab = TAB_SHEET;
                self.refuse(
                    t(
                        "The text you entered is not a valid reference.",
                        "Le texte que vous avez entré ne constitue pas une référence valide.",
                    ),
                    cx,
                );
                return None;
            }
        }
        let rows = parse_titles(self.title_rows.read(cx).text(), true);
        let cols = parse_titles(self.title_cols.read(cx).text(), false);
        match (rows, cols) {
            (Ok(rows), Ok(cols)) => {
                setup.title_rows = rows;
                setup.title_cols = cols;
            }
            _ => {
                self.tab = TAB_SHEET;
                self.refuse(
                    t(
                        "Print titles must be contiguous and complete rows or columns.",
                        "Les titres à imprimer doivent être adjacents et constitués de lignes ou de colonnes entières.",
                    ),
                    cx,
                );
                return None;
            }
        }
        Some(setup)
    }

    /// Store the page setup; false when a field was refused.
    fn apply(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(setup) = self.collect(cx) else {
            cx.notify();
            return false;
        };
        let (sheet, tab) = (self.around.sheet, self.tab);
        self.modal.update_app(cx, |app, cx| {
            let margins = setup.margins;
            let custom =
                margins.preset().is_none() && app.wb.sheets.get(sheet).is_some_and(|s| s.page.margins != margins);
            let ui = app.ext_mut::<PageUi>();
            ui.setup_tab = tab;
            if custom {
                ui.last_custom_margins = Some(margins);
            }
            if app.wb.set_page_setup(sheet, setup) {
                show_breaks(app, sheet);
                app.commit(t("Page Setup", "Mise en page"), cx);
            }
        });
        true
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if self.apply(cx) {
            self.modal.close(window, cx);
        }
    }

    /// "Print…" and "Print Preview": store the setup, then show the Print
    /// screen (unless this dialog was opened from it).
    fn print(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if !self.apply(cx) {
            return;
        }
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if !app.modal_open() {
                print::open(app, window, cx);
            }
        });
    }

    fn check(
        &self,
        id: &'static str,
        text: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        set: impl Fn(&mut PageSetup, bool) + 'static,
    ) -> Checkbox {
        Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
            set(&mut this.setup, *on);
            cx.notify();
        }))
    }

    // ------------------------------------------------------------------ tabs

    fn page_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.around.locale;
        let fit = self.setup.fit_to_page;
        let landscape = self.setup.orientation == Orientation::Landscape;
        let papers = PaperSize::all();
        let paper = self.setup.paper_size(locale);
        let paper_names: Vec<SharedString> = papers.iter().map(|info| info.size.name(locale).into()).collect();
        let mut qualities: Vec<SharedString> = vec![t("Automatic", "Automatique").into()];
        qualities.extend(
            QUALITIES
                .iter()
                .map(|dpi| SharedString::from(format!("{dpi} {}", t("dpi", "ppp")))),
        );
        let quality = match self.setup.horizontal_dpi {
            None => Some(0),
            Some(dpi) => QUALITIES.iter().position(|q| *q == dpi).map(|i| i + 1),
        };
        // The page as it will lie: a sketch beside the two choices.
        let sketch = preview(44.0, 44.0, move |s: &mut Sketch| {
            let (w, h) = if landscape { (36.0, 26.0) } else { (26.0, 36.0) };
            let (x, y) = ((s.width - w) / 2.0, (s.height - h) / 2.0);
            s.rect(x, y, x + w, y + h, c(0xFFFFFF));
            s.frame(x, y, x + w, y + h, 1.0, c(0x808080));
            s.text(
                "A",
                x + w / 2.0 - 5.0,
                y + h / 2.0 - 9.0,
                &widgets::ui_font(),
                14.0,
                c(0x404040),
            );
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                GroupBox::new(t("Orientation", "Orientation")).child(
                    div().flex().items_center().gap(px(16.)).child(sketch).child(
                        RadioGroup::new(
                            "orientation",
                            vec![t("Portrait", "Portrait").into(), t("Landscape", "Paysage").into()],
                            Some(landscape as usize),
                        )
                        .horizontal()
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.setup.orientation = if *ix == 0 {
                                Orientation::Portrait
                            } else {
                                Orientation::Landscape
                            };
                            cx.notify();
                        })),
                    ),
                ),
            )
            .child(
                GroupBox::new(t("Scaling", "Échelle"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                RadioGroup::new(
                                    "adjust",
                                    vec![t("Adjust to:", "Réduire/agrandir à :").into()],
                                    (!fit).then_some(0),
                                )
                                .on_change(cx.listener(|this, _: &usize, _, cx| {
                                    this.setup.fit_to_page = false;
                                    cx.notify();
                                })),
                            )
                            .child(NumberField::new("scale", &self.scale).width(64.0))
                            .child(label(t("% normal size", "% de la taille normale"))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                RadioGroup::new("fit", vec![t("Fit to:", "Ajuster :").into()], fit.then_some(0))
                                    .on_change(cx.listener(|this, _: &usize, _, cx| {
                                        this.setup.fit_to_page = true;
                                        cx.notify();
                                    })),
                            )
                            .child(NumberField::new("fit-width", &self.fit_width).width(56.0))
                            .child(label(t("page(s) wide by", "page(s) en largeur sur")))
                            .child(NumberField::new("fit-height", &self.fit_height).width(56.0))
                            .child(label(t("tall", "en hauteur"))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(labeled(
                        t("Paper size:", "Format du papier :"),
                        Select::new("paper", paper_names, papers.iter().position(|info| info.size == paper))
                            .width(300.0)
                            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                                if let Some(info) = papers.get(*ix) {
                                    this.setup.paper = Some(info.size);
                                    cx.notify();
                                }
                            })),
                    ))
                    .child(labeled(
                        t("Print quality:", "Qualité d'impression :"),
                        Select::new("quality", qualities, quality)
                            .width(140.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                let dpi = ix.checked_sub(1).and_then(|i| QUALITIES.get(i).copied());
                                this.setup.horizontal_dpi = dpi;
                                this.setup.vertical_dpi = dpi;
                                cx.notify();
                            })),
                    ))
                    .child(labeled(
                        t("First page number:", "Commencer la numérotation à :"),
                        TextField::new(&self.first_page).width(70.0),
                    )),
            )
            .into_any_element()
    }

    fn margins_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.around.locale;
        let field = |this: &Self, id: &'static str, text: &'static str, index: usize| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .child(label(text))
                .child(NumberField::new(id, &this.margins[index]).width(72.0))
        };
        // The page with its margins, and where the cells land on it.
        let paper = self.setup.paper_pt(locale);
        let margins = self.typed_margins(cx).unwrap_or(self.setup.margins);
        let (centre_h, centre_v) = (self.setup.center_horizontally, self.setup.center_vertically);
        let diagram = preview(170.0, 190.0, move |s: &mut Sketch| {
            let k = (150.0 / paper.0 as f32).min(170.0 / paper.1 as f32);
            let (w, h) = (paper.0 as f32 * k, paper.1 as f32 * k);
            let (x, y) = ((s.width - w) / 2.0, (s.height - h) / 2.0);
            s.rect(x + 2.0, y + 2.0, x + w + 2.0, y + h + 2.0, c(0xC8C8C8));
            s.rect(x, y, x + w, y + h, c(0xFFFFFF));
            s.frame(x, y, x + w, y + h, 1.0, c(0x808080));
            let inch = 72.0 * k;
            let (left, right) = (x + margins.left as f32 * inch, x + w - margins.right as f32 * inch);
            let (top, bottom) = (y + margins.top as f32 * inch, y + h - margins.bottom as f32 * inch);
            let line = c(0x9A9A9A);
            let dashed = |s: &mut Sketch, vertical: bool, at: f32| {
                let (mut from, to) = if vertical { (y, y + h) } else { (x, x + w) };
                while from < to {
                    let end = (from + 3.0).min(to);
                    if vertical {
                        s.rect(at, from, at + 1.0, end, line);
                    } else {
                        s.rect(from, at, end, at + 1.0, line);
                    }
                    from += 5.0;
                }
            };
            dashed(s, true, left);
            dashed(s, true, right);
            dashed(s, false, top);
            dashed(s, false, bottom);
            dashed(s, false, y + margins.header as f32 * inch);
            dashed(s, false, y + h - margins.footer as f32 * inch);
            // A small table: at the top-left corner of the printable
            // area, or in its middle when centred.
            let (tw, th) = ((right - left).clamp(0.0, 46.0), (bottom - top).clamp(0.0, 34.0));
            let tx = if centre_h { (left + right - tw) / 2.0 } else { left };
            let ty = if centre_v { (top + bottom - th) / 2.0 } else { top };
            if tw > 6.0 && th > 6.0 {
                s.rect(tx, ty, tx + tw, ty + th, c(0xDDEBF7));
                s.frame(tx, ty, tx + tw, ty + th, 1.0, c(0x5B9BD5));
                for i in 1..3 {
                    let lx = tx + tw * i as f32 / 3.0;
                    s.rect(lx, ty, lx + 1.0, ty + th, c(0x5B9BD5));
                    let ly = ty + th * i as f32 / 3.0;
                    s.rect(tx, ly, tx + tw, ly + 1.0, c(0x5B9BD5));
                }
            }
        });
        let unit = if metric(locale) { "cm" } else { t("in", "po") };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap(px(40.))
                    .child(field(self, "top", t("Top:", "Haut :"), 0))
                    .child(field(self, "header", t("Header:", "En-tête :"), 4)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(18.))
                    .child(field(self, "left", t("Left:", "Gauche :"), 2))
                    .child(diagram)
                    .child(field(self, "right", t("Right:", "Droite :"), 3)),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap(px(40.))
                    .child(field(self, "bottom", t("Bottom:", "Bas :"), 1))
                    .child(field(self, "footer", t("Footer:", "Pied de page :"), 5)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(label(t("Center on page:", "Centrer sur la page :")))
                    .child(self.check(
                        "centre-h",
                        t("Horizontally", "Horizontalement"),
                        centre_h,
                        cx,
                        |s, on| s.center_horizontally = on,
                    ))
                    .child(
                        self.check("centre-v", t("Vertically", "Verticalement"), centre_v, cx, |s, on| {
                            s.center_vertically = on
                        }),
                    )
                    .child(div().flex_1())
                    .child(label(format!("({unit})")).text_color(c(theme::TEXT_MUTED))),
            )
            .into_any_element()
    }

    /// The built-in headers or footers as a list, with the sheet's own
    /// added when it is none of them.
    fn preset_list(&self, footer: bool) -> (Vec<String>, Vec<SharedString>, Option<usize>) {
        let hf = &self.setup.header_footer;
        let current = if footer { &hf.odd_footer } else { &hf.odd_header };
        let mut codes = presets(self.around.locale, &views::user_name());
        if !codes.contains(current) {
            codes.push(current.clone());
        }
        let context = self.context();
        let names = codes
            .iter()
            .map(|code| {
                if code.is_empty() {
                    t("(none)", "(aucun)").into()
                } else {
                    SharedString::from(plain_text(code, &context))
                }
            })
            .collect();
        let selected = codes.iter().position(|code| code == current);
        (codes, names, selected)
    }

    /// What a header or footer prints, in a box like the top of a page.
    fn sample(&self, footer: bool) -> impl IntoElement + use<> {
        let hf = &self.setup.header_footer;
        let code = if footer { &hf.odd_footer } else { &hf.odd_header };
        let sections = HfSections::parse(code);
        let context = self.context();
        let texts: Vec<Vec<String>> = [&sections.left, &sections.center, &sections.right]
            .into_iter()
            .map(|section| {
                let text: String = expand(section, &context)
                    .into_iter()
                    .map(|run| match run {
                        HfRun::Text { text, .. } => text,
                        HfRun::Picture => format!("&[{}]", t("Picture", "Image")),
                    })
                    .collect();
                text.lines().map(str::to_string).collect()
            })
            .collect();
        div()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .child(preview(528.0, 48.0, move |s: &mut Sketch| {
                let font = widgets::ui_font();
                for (k, lines) in texts.iter().enumerate() {
                    // Headers hang from the top, footers stand on the bottom.
                    let mut y = if footer {
                        s.height - 4.0 - 14.0 * lines.len() as f32
                    } else {
                        4.0
                    };
                    for line in lines {
                        let width = s.measure(line, &font, 11.0);
                        let x = match k {
                            0 => 6.0,
                            1 => (s.width - width) / 2.0,
                            _ => s.width - 6.0 - width,
                        };
                        s.text(line, x, y, &font, 11.0, c(0x000000));
                        y += 14.0;
                    }
                }
            }))
    }

    fn header_footer_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let hf = &self.setup.header_footer;
        let (odd_even, first, scale, align) = (
            hf.different_odd_even,
            hf.different_first,
            hf.scale_with_doc,
            hf.align_with_margins,
        );
        let (header_codes, header_names, header) = self.preset_list(false);
        let (footer_codes, footer_names, footer) = self.preset_list(true);
        // The two lists line up whatever the length of their labels.
        let row = |text: &'static str, list: Select| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(label(text).w(px(86.)))
                .child(list)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.sample(false))
            .child(row(
                t("Header:", "En-tête :"),
                Select::new("header", header_names, header)
                    .width(440.0)
                    .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                        if let Some(code) = header_codes.get(*ix) {
                            this.setup.header_footer.odd_header = code.clone();
                            cx.notify();
                        }
                    })),
            ))
            .child(
                div()
                    .flex()
                    .justify_center()
                    .gap(px(12.))
                    .py(px(4.))
                    .child(
                        Button::new("custom-header", t("Custom Header…", "En-tête personnalisé…"))
                            .on_click(cx.listener(|this, _, window, cx| this.custom(false, window, cx))),
                    )
                    .child(
                        Button::new("custom-footer", t("Custom Footer…", "Pied de page personnalisé…"))
                            .on_click(cx.listener(|this, _, window, cx| this.custom(true, window, cx))),
                    ),
            )
            .child(row(
                t("Footer:", "Pied de page :"),
                Select::new("footer", footer_names, footer)
                    .width(440.0)
                    .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                        if let Some(code) = footer_codes.get(*ix) {
                            this.setup.header_footer.odd_footer = code.clone();
                            cx.notify();
                        }
                    })),
            ))
            .child(self.sample(true))
            .child(
                div()
                    .pt(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(self.check(
                        "odd-even",
                        t("Different odd and even pages", "Pages paires et impaires différentes"),
                        odd_even,
                        cx,
                        |s, on| s.header_footer.different_odd_even = on,
                    ))
                    .child(self.check(
                        "first",
                        t("Different first page", "Première page différente"),
                        first,
                        cx,
                        |s, on| s.header_footer.different_first = on,
                    ))
                    .child(self.check(
                        "scale-doc",
                        t("Scale with document", "Mettre à l'échelle du document"),
                        scale,
                        cx,
                        |s, on| s.header_footer.scale_with_doc = on,
                    ))
                    .child(self.check(
                        "align-margins",
                        t("Align with page margins", "Aligner sur les marges de page"),
                        align,
                        cx,
                        |s, on| s.header_footer.align_with_margins = on,
                    )),
            )
            .into_any_element()
    }

    /// Open Custom Header or Custom Footer over this dialog.
    fn custom(&mut self, footer: bool, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let hf = self.setup.header_footer.clone();
        let locale = self.around.locale;
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| {
                HfDialog::new(modal, parent, footer, hf, locale, cx)
            });
        });
    }

    fn sheet_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.setup;
        let (gridlines, mono, draft, headings) = (s.gridlines, s.black_and_white, s.draft, s.headings);
        let over_first = s.page_order == PageOrder::OverThenDown;
        let comments = match s.comments {
            PrintComments::None => 0,
            PrintComments::AtEnd => 1,
            PrintComments::AsDisplayed => 2,
        };
        let errors = match s.errors {
            PrintErrors::Displayed => 0,
            PrintErrors::Blank => 1,
            PrintErrors::Dash => 2,
            PrintErrors::NA => 3,
        };
        let names = |list: &[&'static str]| -> Vec<SharedString> { list.iter().map(|name| (*name).into()).collect() };
        // The order the pages are numbered in: an arrow through four pages.
        let order = preview(84.0, 56.0, move |s: &mut Sketch| {
            let ink = c(0x5B9BD5);
            for i in 0..2 {
                for j in 0..2 {
                    let (x, y) = (8.0 + 36.0 * j as f32, 4.0 + 26.0 * i as f32);
                    s.rect(x, y, x + 30.0, y + 22.0, c(0xFFFFFF));
                    s.frame(x, y, x + 30.0, y + 22.0, 1.0, c(0x9A9A9A));
                }
            }
            let centre = |i: usize, j: usize| (23.0 + 36.0 * j as f32, 15.0 + 26.0 * i as f32);
            let path = if over_first {
                [centre(0, 0), centre(0, 1), centre(1, 0), centre(1, 1)]
            } else {
                [centre(0, 0), centre(1, 0), centre(0, 1), centre(1, 1)]
            };
            for pair in path.windows(2) {
                s.line(pair[0], pair[1], 1.5, ink);
            }
            let (ex, ey) = path[3];
            s.rect(ex - 3.0, ey - 3.0, ex + 3.0, ey + 3.0, ink);
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(labeled(
                t("Print area:", "Zone d'impression :"),
                RefEdit::new(&self.print_area, &self.modal),
            ))
            .child(
                GroupBox::new(t("Print titles", "Titres à imprimer"))
                    .child(labeled(
                        t("Rows to repeat at top:", "Lignes à répéter en haut :"),
                        RefEdit::new(&self.title_rows, &self.modal),
                    ))
                    .child(labeled(
                        t("Columns to repeat at left:", "Colonnes à répéter à gauche :"),
                        RefEdit::new(&self.title_cols, &self.modal),
                    )),
            )
            .child(
                GroupBox::new(crate::i18n::t_ctx("group", "Print", "Impression")).child(
                    div()
                        .flex()
                        .gap(px(28.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child(self.check(
                                    "gridlines",
                                    t("Gridlines", "Quadrillage"),
                                    gridlines,
                                    cx,
                                    |s, on| s.gridlines = on,
                                ))
                                .child(self.check(
                                    "mono",
                                    t("Black and white", "En noir et blanc"),
                                    mono,
                                    cx,
                                    |s, on| s.black_and_white = on,
                                ))
                                .child(self.check(
                                    "draft",
                                    t("Draft quality", "Qualité brouillon"),
                                    draft,
                                    cx,
                                    |s, on| s.draft = on,
                                ))
                                .child(self.check(
                                    "headings",
                                    t("Row and column headings", "En-têtes des lignes et des colonnes"),
                                    headings,
                                    cx,
                                    |s, on| s.headings = on,
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(labeled(
                                    t("Comments:", "Commentaires :"),
                                    Select::new(
                                        "comments",
                                        names(&[
                                            t("(None)", "(Aucun)"),
                                            t("At end of sheet", "À la fin de la feuille"),
                                            t("As displayed on sheet", "Tel que sur la feuille"),
                                        ]),
                                        Some(comments),
                                    )
                                    .width(190.0)
                                    .on_change(cx.listener(
                                        |this, ix: &usize, _, cx| {
                                            this.setup.comments = match *ix {
                                                1 => PrintComments::AtEnd,
                                                2 => PrintComments::AsDisplayed,
                                                _ => PrintComments::None,
                                            };
                                            cx.notify();
                                        },
                                    )),
                                ))
                                .child(labeled(
                                    t("Cell errors as:", "Erreurs de cellule comme :"),
                                    Select::new(
                                        "errors",
                                        names(&[t("displayed", "affiché"), t("<blank>", "<vide>"), "--", "#N/A"]),
                                        Some(errors),
                                    )
                                    .width(120.0)
                                    .on_change(cx.listener(
                                        |this, ix: &usize, _, cx| {
                                            this.setup.errors = match *ix {
                                                1 => PrintErrors::Blank,
                                                2 => PrintErrors::Dash,
                                                3 => PrintErrors::NA,
                                                _ => PrintErrors::Displayed,
                                            };
                                            cx.notify();
                                        },
                                    )),
                                )),
                        ),
                ),
            )
            .child(
                GroupBox::new(t("Page order", "Ordre des pages")).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(24.))
                        .child(
                            RadioGroup::new(
                                "order",
                                names(&[
                                    t("Down, then over", "Vers le bas, puis à droite"),
                                    t("Over, then down", "À droite, puis vers le bas"),
                                ]),
                                Some(over_first as usize),
                            )
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.setup.page_order = if *ix == 1 {
                                    PageOrder::OverThenDown
                                } else {
                                    PageOrder::DownThenOver
                                };
                                cx.notify();
                            })),
                        )
                        .child(order),
                ),
            )
            .into_any_element()
    }
}

impl Render for SetupDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = [
            t("Page", "Page"),
            t("Margins", "Marges"),
            t("Header/Footer", "En-tête/Pied de page"),
            t("Sheet", "Feuille"),
        ]
        .iter()
        .map(|name| (*name).into())
        .collect();
        let body = match self.tab {
            TAB_MARGINS => self.margins_tab(cx),
            TAB_HEADER_FOOTER => self.header_footer_tab(cx),
            TAB_SHEET => self.sheet_tab(cx),
            _ => self.page_tab(cx),
        };
        dialog(&self.modal, t("Page Setup", "Mise en page"))
            .width(580.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                TabStrip::new("tabs", tabs, self.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.tab = *ix;
                    cx.notify();
                })),
            )
            .child(div().h(px(352.)).flex().flex_col().child(body))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(Button::new("print", t("Print…", "Imprimer…")).on_click(cx.listener(Self::print)))
                    .child(Button::new("preview", t("Print Preview", "Aperçu")).on_click(cx.listener(Self::print)))
                    // The options of a printer driver: there is none to
                    // open, printing goes through a PDF.
                    .child(Button::new("options", t("Options…", "Options…")).disabled(true)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------- Custom Header / Custom Footer

/// The header (or footer) of the odd pages, then those of the first and
/// of the even pages when the sheet has them.
struct HfDialog {
    modal: Modal,
    parent: WeakEntity<SetupDialog>,
    footer: bool,
    locale: &'static Locale,
    variants: Vec<HfVariant>,
    /// The variant shown (index into `variants`).
    shown: usize,
    /// Left, centre and right text of every variant, as typed.
    texts: Vec<[String; 3]>,
    fields: [Entity<TextInput>; 3],
    /// The field the buttons insert into: the last one that had the
    /// keyboard.
    active: usize,
    pictures: Vec<HfPicture>,
}

impl HfDialog {
    fn new(
        modal: Modal,
        parent: WeakEntity<SetupDialog>,
        footer: bool,
        hf: HeaderFooter,
        locale: &'static Locale,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut variants = vec![HfVariant::Odd];
        if hf.different_first {
            variants.push(HfVariant::First);
        }
        if hf.different_odd_even {
            variants.push(HfVariant::Even);
        }
        let texts: Vec<[String; 3]> = variants
            .iter()
            .map(|variant| {
                let sections = HfSections::parse(if footer {
                    hf.footer(*variant)
                } else {
                    hf.header(*variant)
                });
                [&sections.left, &sections.center, &sections.right].map(|code| to_dialog_text(code, locale))
            })
            .collect();
        let fields = [0, 1, 2].map(|k| {
            let field = widgets::input(cx, texts[0][k].clone());
            // The buttons insert at the caret: coming back from a dialog
            // must not select the whole section.
            field.update(cx, |field, _| field.select_on_tab = false);
            cx.subscribe(&field, move |this: &mut Self, _, event: &InputEvent, _| {
                if matches!(event, InputEvent::Focused | InputEvent::Clicked) {
                    this.active = k;
                }
            })
            .detach();
            field
        });
        HfDialog {
            modal,
            parent,
            footer,
            locale,
            variants,
            shown: 0,
            texts,
            fields,
            active: 0,
            pictures: hf.pictures,
        }
    }

    fn keep_fields(&mut self, cx: &App) {
        for k in 0..3 {
            self.texts[self.shown][k] = self.fields[k].read(cx).text().to_string();
        }
    }

    fn show(&mut self, variant: usize, cx: &mut Context<Self>) {
        self.keep_fields(cx);
        self.shown = variant.min(self.variants.len() - 1);
        for k in 0..3 {
            let text = self.texts[self.shown][k].clone();
            self.fields[k].update(cx, |field, cx| field.set_text(text, cx));
        }
        cx.notify();
    }

    /// Insert at the caret of the section last used.
    fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.fields[self.active].clone();
        window.focus(&field.read(cx).focus_handle(cx));
        field.update(cx, |field, cx| field.insert(text, window, cx));
    }

    fn insert_field(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = to_dialog_text(code, self.locale);
        self.insert(&text, window, cx);
    }

    fn slot(&self) -> HfSlot {
        HfSlot {
            variant: self.variants[self.shown],
            footer: self.footer,
            section: [HfSection::Left, HfSection::Center, HfSection::Right][self.active],
        }
    }

    fn insert_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let slot = self.slot();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(data) = paths.into_iter().next().and_then(|path| std::fs::read(path).ok()) else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let Some(info) = image_info(&data) else {
                    return;
                };
                let (width, height) = info.natural_size();
                this.pictures.retain(|p| p.slot != slot);
                this.pictures.push(HfPicture {
                    slot,
                    data: Arc::from(data),
                    size: (width * 0.75, height * 0.75),
                });
                // One picture per section: the field names it once.
                let field = &this.fields[this.active];
                if !from_dialog_text(field.read(cx).text(), this.locale).contains("&G") {
                    this.insert_field("&G", window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn format_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let locale = self.locale;
        self.modal.update_app(cx, |app, cx| {
            let family = app.wb.styles.get(opensheet_engine::StyleId(0)).font.name.to_string();
            app.open_modal(window, cx, move |modal, _, cx| {
                FontDialog::new(modal, parent, family, locale, cx)
            });
        });
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.keep_fields(cx);
        let (footer, locale) = (self.footer, self.locale);
        let codes: Vec<(HfVariant, String)> = self
            .variants
            .iter()
            .zip(&self.texts)
            .map(|(variant, texts)| {
                let [left, center, right] = texts.clone().map(|text| from_dialog_text(&text, locale));
                (*variant, HfSections { left, center, right }.to_code())
            })
            .collect();
        let pictures = self.pictures.clone();
        self.parent
            .update(cx, |parent, cx| {
                let hf = &mut parent.setup.header_footer;
                for (variant, code) in codes {
                    *(if footer {
                        hf.footer_mut(variant)
                    } else {
                        hf.header_mut(variant)
                    }) = code;
                }
                hf.pictures = pictures;
                cx.notify();
            })
            .ok();
        self.modal.close(window, cx);
    }
}

impl Render for HfDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = |id: &'static str, icon_name: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .bg(c(0xFFFFFF))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(icon(icon_name, 15.0, theme::TEXT))
                .tooltip(tooltip(tip))
        };
        let field = |cx: &mut Context<Self>,
                     id: &'static str,
                     icon_name: &'static str,
                     tip: &'static str,
                     code: &'static str| {
            tool(id, icon_name, tip)
                .on_click(cx.listener(move |this, _, window, cx| this.insert_field(code, window, cx)))
        };
        let tools = div()
            .flex()
            .gap(px(4.))
            .child(
                tool("font", "page-font", t("Format Text", "Mettre le texte en forme"))
                    .on_click(cx.listener(|this, _, window, cx| this.format_text(window, cx))),
            )
            .child(field(
                cx,
                "page",
                "page-number",
                t("Insert Page Number", "Insérer le numéro de page"),
                "&P",
            ))
            .child(field(
                cx,
                "pages",
                "page-count",
                t("Insert Number of Pages", "Insérer le nombre de pages"),
                "&N",
            ))
            .child(field(
                cx,
                "date",
                "page-date",
                t("Insert Date", "Insérer la date"),
                "&D",
            ))
            .child(field(cx, "time", "clock", t("Insert Time", "Insérer l'heure"), "&T"))
            .child(field(
                cx,
                "path",
                "folder",
                t("Insert File Path", "Insérer le chemin du fichier"),
                "&Z&F",
            ))
            .child(field(
                cx,
                "file",
                "book",
                t("Insert File Name", "Insérer le nom du fichier"),
                "&F",
            ))
            .child(field(
                cx,
                "tab",
                "table",
                t("Insert Sheet Name", "Insérer le nom de la feuille"),
                "&A",
            ))
            .child(
                tool("picture", "picture", t("Insert Picture", "Insérer une image"))
                    .on_click(cx.listener(|this, _, window, cx| this.insert_picture(window, cx))),
            );
        let variant_names: Vec<SharedString> = self
            .variants
            .iter()
            .map(|variant| {
                let (header, footer) = match variant {
                    HfVariant::Odd if self.variants.len() == 1 => (t("Header", "En-tête"), t("Footer", "Pied de page")),
                    HfVariant::Odd => (
                        t("Odd Page Header", "En-tête de page impaire"),
                        t("Odd Page Footer", "Pied de page impaire"),
                    ),
                    HfVariant::First => (
                        t("First Page Header", "En-tête de première page"),
                        t("First Page Footer", "Pied de page de première page"),
                    ),
                    HfVariant::Even => (
                        t("Even Page Header", "En-tête de page paire"),
                        t("Even Page Footer", "Pied de page paire"),
                    ),
                };
                SharedString::from(if self.footer { footer } else { header })
            })
            .collect();
        let section = |this: &Self, text: &'static str, k: usize| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(label(text))
                .child(TextField::new(&this.fields[k]))
        };
        let title = if self.footer {
            t("Footer", "Pied de page")
        } else {
            t("Header", "En-tête")
        };
        dialog(&self.modal, title)
            .width(620.0)
            .on_ok(cx.listener(Self::ok))
            .when(self.variants.len() > 1, |d| {
                d.child(TabStrip::new("variants", variant_names, self.shown).on_change(cx.listener(
                    |this, ix: &usize, _, cx| this.show(*ix, cx),
                )))
            })
            .child(
                div().text_color(c(theme::TEXT_MUTED)).line_height(px(17.)).child(t(
                    "To customize the header or footer, place the cursor in the section, and then choose the content you want to insert.",
                    "Pour personnaliser l'en-tête ou le pied de page, placez le curseur dans la section, puis choisissez le contenu à insérer.",
                )),
            )
            .child(tools)
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(section(self, t("Left section:", "Section de gauche :"), 0))
                    .child(section(self, t("Center section:", "Section centrale :"), 1))
                    .child(section(self, t("Right section:", "Section de droite :"), 2)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------------- the font of a text

/// The words of the font styles, as Excel writes them in a header.
fn style_names(locale: &Locale) -> [&'static str; 4] {
    match locale.id {
        "fr" => ["Normal", "Italique", "Gras", "Gras italique"],
        "es" => ["Normal", "Cursiva", "Negrita", "Negrita cursiva"],
        "de" => ["Standard", "Kursiv", "Fett", "Fett kursiv"],
        "it" => ["Normale", "Corsivo", "Grassetto", "Grassetto corsivo"],
        _ => ["Regular", "Italic", "Bold", "Bold Italic"],
    }
}

/// The codes that set a font in a header or footer:
/// `&"Arial,Bold"&14&U`.
pub(crate) fn font_code(
    family: &str,
    style: &str,
    size: Option<u32>,
    underline: usize,
    strike: bool,
    script: usize,
) -> String {
    let mut code = format!("&\"{},{}\"", family.replace('"', ""), style);
    if let Some(size) = size {
        code.push_str(&format!("&{size}"));
    }
    match underline {
        1 => code.push_str("&U"),
        2 => code.push_str("&E"),
        _ => {}
    }
    if strike {
        code.push_str("&S");
    }
    match script {
        1 => code.push_str("&X"),
        2 => code.push_str("&Y"),
        _ => {}
    }
    code
}

/// "Format Text" of the Header and Footer dialogs: what is typed after
/// the caret is set in this font.
struct FontDialog {
    modal: Modal,
    parent: WeakEntity<HfDialog>,
    locale: &'static Locale,
    family: Entity<TextInput>,
    families: Vec<SharedString>,
    style: usize,
    size: Entity<TextInput>,
    underline: usize,
    strike: bool,
    /// None, superscript, subscript.
    script: usize,
}

impl FontDialog {
    fn new(
        modal: Modal,
        parent: WeakEntity<HfDialog>,
        family: String,
        locale: &'static Locale,
        cx: &mut Context<Self>,
    ) -> Self {
        FontDialog {
            modal,
            parent,
            locale,
            family: widgets::input(cx, family),
            families: FontDb::system()
                .families()
                .into_iter()
                .map(SharedString::from)
                .collect(),
            style: 0,
            size: widgets::number_input(cx, Some(11.0), 1.0, 409.0, 1.0),
            underline: 0,
            strike: false,
            script: 0,
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let size = self.size.read(cx).number().map(|n| n.round().clamp(1.0, 409.0) as u32);
        let code = font_code(
            self.family.read(cx).text().trim(),
            style_names(self.locale)[self.style],
            size,
            self.underline,
            self.strike,
            self.script,
        );
        self.modal.close(window, cx);
        self.parent
            .update(cx, |parent, cx| parent.insert(&code, window, cx))
            .ok();
    }
}

impl Render for FontDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names = |list: &[&'static str]| -> Vec<SharedString> { list.iter().map(|name| (*name).into()).collect() };
        let sizes: Vec<SharedString> = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72]
            .iter()
            .map(|size| SharedString::from(size.to_string()))
            .collect();
        dialog(&self.modal, t("Font", "Police"))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Font:", "Police :")))
                            .child(ComboBox::new("family", &self.family, self.families.clone()).width(180.0)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Font style:", "Style :")))
                            .child(
                                Select::new("style", names(&style_names(self.locale)), Some(self.style))
                                    .width(120.0)
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        this.style = *ix;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Size:", "Taille :")))
                            .child(ComboBox::new("size", &self.size, sizes).width(64.0)),
                    ),
            )
            .child(labeled(
                t("Underline:", "Soulignement :"),
                Select::new(
                    "underline",
                    names(&[t("None", "Aucun"), t("Single", "Simple"), t("Double", "Double")]),
                    Some(self.underline),
                )
                .width(120.0)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.underline = *ix;
                    cx.notify();
                })),
            ))
            .child(
                GroupBox::new(t("Effects", "Effets"))
                    .child(
                        Checkbox::new("strike", t("Strikethrough", "Barré"), self.strike).on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.strike = *on;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Checkbox::new("super", t("Superscript", "Exposant"), self.script == 1).on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.script = if *on { 1 } else { 0 };
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Checkbox::new("sub", t("Subscript", "Indice"), self.script == 2).on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.script = if *on { 2 } else { 0 };
                                cx.notify();
                            },
                        )),
                    ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_area_field() {
        let (en, fr) = (Locale::en(), Locale::fr());
        let ranges = vec![Rect::new(0, 0, 8, 2), Rect::new(0, 4, 8, 5)];
        assert_eq!(area_text(&ranges, en), "$A$1:$C$9,$E$1:$F$9");
        assert_eq!(area_text(&ranges, fr), "$A$1:$C$9;$E$1:$F$9");
        assert_eq!(parse_area("$A$1:$C$9;$E$1:$F$9", fr), Some(ranges.clone()));
        assert_eq!(parse_area("a1:c9 , Feuil1!E1:F9", fr), Some(ranges));
        assert_eq!(parse_area("", en), Some(Vec::new()));
        assert_eq!(parse_area("B:B", en), Some(vec![Rect::new(0, 1, MAX_ROWS - 1, 1)]));
        assert_eq!(parse_area("A1:", en), None);
        assert_eq!(parse_area("Budget", en), None);
    }

    #[test]
    fn print_title_fields() {
        assert_eq!(titles_text(Some((0, 2)), true), "$1:$3");
        assert_eq!(titles_text(Some((0, 1)), false), "$A:$B");
        assert_eq!(titles_text(None, true), "");
        assert_eq!(parse_titles("$1:$3", true), Ok(Some((0, 2))));
        assert_eq!(parse_titles("=Sheet1!$A:$B", false), Ok(Some((0, 1))));
        // Cells picked on the sheet give their rows or their columns.
        assert_eq!(parse_titles("$B$2:$D$3", true), Ok(Some((1, 2))));
        assert_eq!(parse_titles("$B$2:$D$3", false), Ok(Some((1, 3))));
        assert_eq!(parse_titles("  ", true), Ok(None));
        // Columns where rows are asked for.
        assert_eq!(parse_titles("$A:$B", true), Err(()));
        assert_eq!(parse_titles("$1:$2", false), Err(()));
        assert_eq!(parse_titles("titles", true), Err(()));
    }

    #[test]
    fn first_page_number_field() {
        assert_eq!(parse_first_page("Auto"), Ok(None));
        assert_eq!(parse_first_page(""), Ok(None));
        assert_eq!(parse_first_page(" 5 "), Ok(Some(5)));
        assert_eq!(parse_first_page("-2"), Ok(Some(-2)));
        assert_eq!(parse_first_page("two"), Err(()));
        assert_eq!(first_page_text(Some(7)), "7");
    }

    #[test]
    fn font_codes() {
        assert_eq!(font_code("Arial", "Bold", Some(14), 0, false, 0), "&\"Arial,Bold\"&14");
        assert_eq!(
            font_code("Calibri", "Gras italique", None, 2, true, 1),
            "&\"Calibri,Gras italique\"&E&S&X"
        );
        assert_eq!(
            font_code("Times", "Regular", Some(9), 1, false, 2),
            "&\"Times,Regular\"&9&U&Y"
        );
        // The engine reads what the dialog writes.
        let code = format!(
            "{}Total",
            font_code("Arial", style_names(Locale::fr())[3], Some(14), 0, false, 0)
        );
        let context = HfContext {
            page: 1,
            pages: 1,
            now: 0.0,
            file_name: "",
            file_path: "",
            sheet_name: "",
            locale: Locale::fr(),
        };
        match &expand(&code, &context)[0] {
            HfRun::Text { text, format } => {
                assert_eq!(text, "Total");
                assert!(format.bold && format.italic);
                assert_eq!((format.size, format.font.as_deref()), (14.0, Some("Arial")));
            }
            HfRun::Picture => panic!("a text run was expected"),
        }
    }
}
