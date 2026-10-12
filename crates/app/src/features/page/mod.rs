//! Page setup, the Page Layout view, the Page Break Preview and printing.
//!
//! * this file: the commands of the Page Layout tab and their drop-downs,
//!   and the hooks that hand the grid over to the views;
//! * `views.rs`: what the three views draw on the grid (page breaks, the
//!   blue outlines of Page Break Preview, the pages of the Page Layout
//!   view with their rulers, headers and footers) and what the mouse does
//!   there;
//! * `setup.rs`: the Page Setup dialog and the Header / Footer dialogs;
//! * `print.rs`: the Print screen (preview and settings), PDF output and
//!   the printers of this machine;
//! * `scene.rs`: painting a printed page (`opensheet_print::Scene`) in a
//!   window, for the preview;
//! * `paper.rs`: what the application paints on paper itself (charts,
//!   pictures and shapes, data bars and icons).

mod paper;
mod print;
mod scene;
mod setup;
mod views;

use super::{
    Command, ContextMenu, ContextTab, Cx, Feature, MenuSection, MenuTarget, Painter, StatusItem, ViewMode, Zone,
    ZoneEvent, ZoneResponse,
};
use crate::app::{ExcelApp, MenuAction, MenuEntry};
use crate::grid::{Axis, GridLayout};
use crate::i18n::t;
use crate::shell::Alert;
use crate::theme::{self, c};
use crate::widgets::icon;
use gpui::{AnyElement, FontWeight, SharedString, WeakEntity, Window, div, point, prelude::*, px};
use opensheet_engine::page::{MarginPreset, Margins, Orientation, PageSetup, PaperSize, breaks_for_selection};
use opensheet_engine::{Locale, Rect};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub struct Page;

/// The contextual tab shown while a header or a footer is typed.
const HF_TAB: &str = "header-footer";

commands! {
    /// What the user can ask of page setup and printing.
    pub enum Cmd {
        Print = "print", t("Print…", "Imprimer…");
        Setup = "page", t("Page Setup", "Mise en page");
        SetupSheet = "page", t("Sheet Options", "Options de la feuille de calcul");
        Margins = "margins", t("Margins", "Marges");
        MarginsLast = "", t("Last Custom Setting", "Dernier paramètre personnalisé");
        MarginsNormal = "", crate::i18n::t_ctx("margins", "Normal", "Normales");
        MarginsWide = "", t("Wide", "Larges");
        MarginsNarrow = "", t("Narrow", "Étroites");
        CustomMargins = "", t("Custom Margins…", "Marges personnalisées…");
        Orientation = "page", t("Orientation", "Orientation");
        Portrait = "", t("Portrait", "Portrait");
        Landscape = "", t("Landscape", "Paysage");
        Size = "page", t("Size", "Taille");
        MoreSizes = "", t("More Paper Sizes…", "Autres tailles de papier…");
        PrintArea = "print", t("Print Area", "ZoneImpr");
        SetPrintArea = "", t("Set Print Area", "Définir");
        ClearPrintArea = "", t("Clear Print Area", "Annuler");
        AddToPrintArea = "", t("Add to Print Area", "Ajouter à la zone d'impression");
        Breaks = "breaks", t("Breaks", "Sauts de page");
        InsertBreak = "", t("Insert Page Break", "Insérer un saut de page");
        RemoveBreak = "", t("Remove Page Break", "Supprimer le saut de page");
        ResetBreaks = "", t("Reset All Page Breaks", "Rétablir tous les sauts de page");
        Background = "picture", t("Background", "Arrière-plan");
        PrintTitles = "table", t("Print Titles", "Imprimer les titres");
        FitWidth = "columns", t("Width", "Largeur");
        FitHeight = "align-middle", t("Height", "Hauteur");
        Scale = "zoom", t("Scale", "Échelle");
        PrintGridlines = "", t("Print", "Imprimer");
        PrintHeadings = "", t("Print", "Imprimer");
        Ruler = "", t("Ruler", "Règle");
        CustomViews = "eye", t("Custom Views", "Affichages personnalisés");
        HeaderFooter = "header-footer", t("Header & Footer", "En-tête et pied de page");
        HfHeader = "header-footer", t("Header", "En-tête");
        HfFooter = "header-footer", t("Footer", "Pied de page");
        HfPageNumber = "page-number", t("Page Number", "Numéro de page");
        HfPages = "page-count", t("Number of Pages", "Nombre de pages");
        HfDate = "page-date", t("Current Date", "Date actuelle");
        HfTime = "clock", t("Current Time", "Heure actuelle");
        HfPath = "folder", t("File Path", "Chemin d'accès");
        HfFile = "book", t("File Name", "Nom de fichier");
        HfSheet = "table", t("Sheet Name", "Nom de la feuille");
        HfPicture = "picture", t("Picture", "Image");
        HfFormatPicture = "picture", t("Format Picture", "Format de l'image");
        HfGoHeader = "header-footer", t("Go to Header", "Atteindre l'en-tête");
        HfGoFooter = "header-footer", t("Go to Footer", "Atteindre le pied de page");
        HfDifferentFirst = "", t("Different First Page", "Première page différente");
        HfDifferentOddEven = "", t("Different Odd & Even Pages", "Pages paires et impaires différentes");
        HfScaleWithDoc = "", t("Scale with Document", "Mettre à l'échelle du document");
        HfAlignMargins = "", t("Align with Page Margins", "Aligner sur les marges de page");
        ResetPrintArea = "", t("Reset Print Area", "Rétablir la zone d'impression");
        PageSetupMenu = "page", t("Page Setup…", "Mise en page…");
    }
}

/// What the feature remembers between events.
#[derive(Default)]
pub(crate) struct PageUi {
    /// Names of the sheets whose page breaks show in Normal view: those
    /// that were previewed or printed, or whose setup was changed.
    breaks_shown: HashSet<String>,
    /// The view each sheet was last seen in, by name.
    last_mode: HashMap<String, ViewMode>,
    /// The margins last set by hand, for "Last Custom Setting".
    last_custom_margins: Option<Margins>,
    ruler_hidden: bool,
    /// The margins between the pages of the Page Layout view are folded
    /// away ("Hide White Space").
    white_space_hidden: bool,
    /// Zoom of the Page Break Preview and of the other views (0 = not
    /// used yet), and which of the two the window shows.
    preview_zoom: f32,
    normal_zoom: f32,
    zoom_is_preview: bool,
    /// The tab the Page Setup dialog was last left on.
    setup_tab: usize,
    /// Printer and options last used in the Print screen.
    print: print::Memory,
    /// A page break or an edge of the print area being dragged.
    drag: Option<views::BreakDrag>,
    /// The header or footer being typed in the Page Layout view.
    hf: Option<views::HfEdit>,
    /// The pages of the current sheet, kept until the workbook changes.
    cache: RefCell<Option<views::Cached>>,
}

/// Lengths are shown in inches where Excel is in English and in
/// centimetres elsewhere.
pub(crate) fn metric(locale: &Locale) -> bool {
    PaperSize::default_for(locale) != PaperSize::LETTER
}

/// A length stored in inches, in the unit of the locale.
pub(crate) fn to_unit(inches: f64, locale: &Locale) -> f64 {
    if metric(locale) { inches * 2.54 } else { inches }
}

pub(crate) fn from_unit(value: f64, locale: &Locale) -> f64 {
    if metric(locale) { value / 2.54 } else { value }
}

/// `0.75"` or `1,91 cm`.
pub(crate) fn length_text(inches: f64, locale: &Locale) -> String {
    let number = |v: f64| {
        let text = format!("{v:.2}");
        let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        text.replace('.', &locale.decimal_sep.to_string())
    };
    if metric(locale) {
        format!("{} cm", number(inches * 2.54))
    } else {
        format!("{}\"", number(inches))
    }
}

/// Change the page setup of the current sheet as one undo step. The
/// sheet's page breaks show in Normal view from then on, as in Excel.
pub(crate) fn change(app: &mut ExcelApp, label: &str, cx: &mut Cx, f: impl FnOnce(&mut PageSetup)) -> bool {
    let changed = app.wb.update_page_setup(app.sheet, f);
    if changed {
        show_breaks(app, app.sheet);
        app.commit(label, cx);
    }
    changed
}

/// From now on the page breaks of a sheet are drawn in Normal view.
pub(crate) fn show_breaks(app: &mut ExcelApp, sheet: usize) {
    if let Some(name) = app.wb.sheets.get(sheet).map(|s| s.name.clone()) {
        app.ext_mut::<PageUi>().breaks_shown.insert(name);
    }
}

fn setup(app: &ExcelApp) -> &PageSetup {
    &app.wb.sheets[app.sheet].page
}

// ------------------------------------------------------------- drop-downs

/// One entry of a [`Gallery`].
struct GalleryItem {
    title: SharedString,
    /// Details under the title (the values of a margin preset).
    lines: Vec<SharedString>,
    checked: bool,
    action: MenuAction,
}

/// A drop-down whose entries have a title and lines of details, like the
/// Margins and Size galleries of the ribbon; `footer` holds the plain
/// entries under them.
struct Gallery {
    app: WeakEntity<ExcelApp>,
    width: f32,
    items: Vec<GalleryItem>,
    footer: Vec<(SharedString, MenuAction)>,
}

impl Gallery {
    fn run(&self, action: &MenuAction, window: &mut Window, cx: &mut gpui::App) {
        let action = action.clone();
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                action(app, window, cx);
            })
            .ok();
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            rows.push(
                div()
                    .id(("gallery-item", i))
                    .flex()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(5.))
                    .mx(px(4.))
                    .rounded(px(4.))
                    .when(item.checked, |d| d.bg(c(theme::TOGGLED)))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(
                        div()
                            .w(px(18.))
                            .pt(px(1.))
                            .when(item.checked, |d| d.child(icon("check", 14.0, theme::ACCENT))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(item.title.clone()))
                            .children(item.lines.iter().map(|line| {
                                div()
                                    .text_size(px(11.5))
                                    .text_color(c(theme::TEXT_MUTED))
                                    .whitespace_nowrap()
                                    .child(line.clone())
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(action) = this.items.get(i).map(|item| item.action.clone()) {
                            this.run(&action, window, cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        if !self.footer.is_empty() {
            rows.push(div().h(px(1.)).my_1().mx_2().bg(c(theme::BORDER)).into_any_element());
        }
        for (i, (label, _)) in self.footer.iter().enumerate() {
            rows.push(
                div()
                    .id(("gallery-footer", i))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .pl(px(34.))
                    .pr(px(8.))
                    .mx(px(4.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(label.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(action) = this.footer.get(i).map(|(_, action)| action.clone()) {
                            this.run(&action, window, cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        div()
            .id("gallery")
            .w(px(self.width))
            .max_h(px(520.))
            .overflow_y_scroll()
            .py_1()
            .text_size(px(12.5))
            .children(rows)
    }
}

/// Show a gallery under the ribbon control whose command is running.
fn open_gallery(
    app: &mut ExcelApp,
    width: f32,
    items: Vec<GalleryItem>,
    footer: Vec<(SharedString, MenuAction)>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let anchor = app.command_anchor.and_then(|id| app.anchors.borrow().get(id).copied());
    let pos = match anchor {
        Some(bounds) => point(bounds.origin.x, bounds.origin.y + bounds.size.height + px(2.)),
        None => window.mouse_position(),
    };
    let weak = cx.weak_entity();
    let view = cx.new(|_| Gallery {
        app: weak,
        width,
        items,
        footer,
    });
    app.open_popup_view(pos, view, cx);
}

fn command_action(cmd: Cmd) -> MenuAction {
    Rc::new(move |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| cmd.run(app, window, cx))
}

fn margins_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    let current = setup(app).margins;
    let lines = |m: &Margins| -> Vec<SharedString> {
        let pair = |a: &str, x: f64, b: &str, y: f64| -> SharedString {
            format!(
                "{}    {}",
                crate::i18n::labelled(a, length_text(x, locale)),
                crate::i18n::labelled(b, length_text(y, locale))
            )
            .into()
        };
        vec![
            pair(t("Top", "Haut"), m.top, t("Bottom", "Bas"), m.bottom),
            pair(
                t("Left", "Gauche"),
                m.left,
                crate::i18n::t_ctx("margin", "Right", "Droite"),
                m.right,
            ),
            pair(t("Header", "En-tête"), m.header, t("Footer", "Pied de page"), m.footer),
        ]
    };
    let mut items = Vec::new();
    let last = app.ext::<PageUi>().and_then(|ui| ui.last_custom_margins);
    if let Some(last) = last {
        items.push(GalleryItem {
            title: Cmd::MarginsLast.label().into(),
            lines: lines(&last),
            checked: current.preset().is_none() && current == last,
            action: command_action(Cmd::MarginsLast),
        });
    }
    for (cmd, preset) in [
        (Cmd::MarginsNormal, MarginPreset::Normal),
        (Cmd::MarginsWide, MarginPreset::Wide),
        (Cmd::MarginsNarrow, MarginPreset::Narrow),
    ] {
        items.push(GalleryItem {
            title: cmd.label().into(),
            lines: lines(&Margins::from_preset(preset)),
            checked: current.preset() == Some(preset),
            action: command_action(cmd),
        });
    }
    let footer = vec![(Cmd::CustomMargins.label().into(), command_action(Cmd::CustomMargins))];
    open_gallery(app, 330.0, items, footer, window, cx);
}

fn size_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    let current = setup(app).paper_size(locale);
    let items = PaperSize::COMMON
        .iter()
        .map(|paper| {
            let paper = *paper;
            GalleryItem {
                title: paper.name(locale).into(),
                lines: vec![paper.dimensions_text(locale).into()],
                checked: paper == current,
                action: Rc::new(move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
                    change(app, t("Paper Size", "Format du papier"), cx, |s| s.paper = Some(paper));
                }),
            }
        })
        .collect();
    let footer = vec![(Cmd::MoreSizes.label().into(), command_action(Cmd::MoreSizes))];
    open_gallery(app, 250.0, items, footer, window, cx);
}

/// The entries of the Width and Height drop-downs of Scale to Fit.
fn fit_menu(app: &ExcelApp, width: bool) -> Vec<MenuEntry> {
    let s = setup(app);
    let current = match (s.fit_to_page, width) {
        (false, _) => 0,
        (true, true) => s.fit_width,
        (true, false) => s.fit_height,
    };
    let entry = |label: String, pages: u16| {
        MenuEntry::new(label, move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
            change(app, t("Scale to Fit", "Mise à l'échelle"), cx, |s| {
                // Both counts are automatic until "Fit to" is on.
                let (mut w, mut h) = if s.fit_to_page {
                    (s.fit_width, s.fit_height)
                } else {
                    (0, 0)
                };
                if width {
                    w = pages;
                } else {
                    h = pages;
                }
                s.fit_to_page = w != 0 || h != 0;
                if s.fit_to_page {
                    (s.fit_width, s.fit_height) = (w, h);
                }
            });
        })
        .checked(current == pages)
    };
    let mut entries = vec![entry(t("Automatic", "Automatique").to_string(), 0)];
    for pages in 1..=9u16 {
        let word = if pages == 1 {
            t("page", "page")
        } else {
            t("pages", "pages")
        };
        entries.push(entry(format!("{pages} {word}"), pages));
    }
    entries.push(MenuEntry::separator());
    entries.push(MenuEntry::new(
        t("More Pages…", "Autres pages…"),
        |app, window, cx| setup::open(app, setup::TAB_PAGE, window, cx),
    ));
    entries
}

/// The entries of the Scale drop-down: Excel has a spin box there, which
/// is out of use while a number of pages is asked for.
fn scale_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let s = setup(app);
    let (fit, current) = (s.fit_to_page, s.scale);
    let mut values = vec![25u16, 50, 75, 90, 100, 110, 125, 150, 200, 400];
    if !values.contains(&current) {
        values.push(current);
        values.sort_unstable();
    }
    let mut entries: Vec<MenuEntry> = values
        .into_iter()
        .map(|percent| {
            MenuEntry::new(
                format!("{percent} %"),
                move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
                    change(app, t("Scale", "Échelle"), cx, |s| s.scale = percent);
                },
            )
            .checked(!fit && percent == current)
            .disabled(fit)
        })
        .collect();
    entries.push(MenuEntry::separator());
    entries.push(MenuEntry::new(
        t("Custom Scaling Options…", "Options de mise à l'échelle personnalisée…"),
        |app, window, cx| setup::open(app, setup::TAB_PAGE, window, cx),
    ));
    entries
}

fn set_print_area(app: &mut ExcelApp, cx: &mut Cx) {
    let ranges = app.sel().ranges.clone();
    let apply = move |app: &mut ExcelApp, cx: &mut Cx| {
        if app.wb.set_print_area(app.sheet, &ranges) {
            show_breaks(app, app.sheet);
            app.commit(
                crate::i18n::t_ctx("undo", "Set Print Area", "Définir la zone d'impression"),
                cx,
            );
        }
    };
    if app.sel().is_single_cell()
        && app.wb.sheets[app.sheet]
            .merge_at(app.active().0, app.active().1)
            .is_none()
    {
        // Excel asks before printing a single cell.
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "You've selected a single cell for the print area.\n\n• If this is correct, click OK.\n• If you selected a single cell by mistake, click Cancel, select the cells you want to include, and then click Set Print Area again.",
                    "La zone d'impression que vous avez définie ne comporte qu'une seule cellule.\n\n• Si cela est correct, cliquez sur OK.\n• Si vous n'avez sélectionné qu'une cellule par erreur, cliquez sur Annuler, sélectionnez les cellules à inclure puis cliquez à nouveau sur Zone d'Impression Définir.",
                ),
            )
            .warning()
            .button(t("OK", "OK"), move |app, _, cx| apply(app, cx))
            .close_button(t("Cancel", "Annuler")),
            cx,
        );
    } else {
        apply(app, cx);
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        let s = setup(app);
        let sel = app.sel().current();
        match self {
            // Sheet backgrounds are not modelled: the picture would have
            // nowhere to go.
            Cmd::Background => false,
            // As in Excel, the ruler belongs to the Page Layout view.
            Cmd::Ruler => app.view_mode() == ViewMode::PageLayout,
            Cmd::ClearPrintArea | Cmd::ResetPrintArea => !s.print_area.is_empty(),
            Cmd::AddToPrintArea => !s.print_area.is_empty() && !s.print_area.iter().any(|r| r.intersects(&sel)),
            Cmd::RemoveBreak => s.row_breaks.contains(&sel.r0) || s.col_breaks.contains(&sel.c0),
            Cmd::ResetBreaks => !s.row_breaks.is_empty() || !s.col_breaks.is_empty(),
            Cmd::InsertBreak => breaks_for_selection(sel) != (None, None),
            Cmd::MarginsLast => app.ext::<PageUi>().is_some_and(|ui| ui.last_custom_margins.is_some()),
            Cmd::HfPageNumber
            | Cmd::HfPages
            | Cmd::HfDate
            | Cmd::HfTime
            | Cmd::HfPath
            | Cmd::HfFile
            | Cmd::HfSheet
            | Cmd::HfPicture
            | Cmd::HfGoHeader
            | Cmd::HfGoFooter => views::editing_header_footer(app),
            Cmd::HfFormatPicture => views::edited_picture(app).is_some(),
            _ => true,
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let s = setup(app);
        match self {
            Cmd::PrintGridlines => s.gridlines,
            Cmd::PrintHeadings => s.headings,
            Cmd::Ruler => {
                app.view_mode() == ViewMode::PageLayout && !app.ext::<PageUi>().is_some_and(|ui| ui.ruler_hidden)
            }
            Cmd::Portrait => s.orientation == Orientation::Portrait,
            Cmd::Landscape => s.orientation == Orientation::Landscape,
            Cmd::MarginsNormal => s.margins.preset() == Some(MarginPreset::Normal),
            Cmd::MarginsWide => s.margins.preset() == Some(MarginPreset::Wide),
            Cmd::MarginsNarrow => s.margins.preset() == Some(MarginPreset::Narrow),
            Cmd::HfDifferentFirst => s.header_footer.different_first,
            Cmd::HfDifferentOddEven => s.header_footer.different_odd_even,
            Cmd::HfScaleWithDoc => s.header_footer.scale_with_doc,
            Cmd::HfAlignMargins => s.header_footer.align_with_margins,
            _ => false,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        let list = |app: &ExcelApp, commands: &[Cmd]| -> Vec<MenuEntry> {
            commands.iter().map(|cmd| MenuEntry::command(*cmd, app)).collect()
        };
        let margins = |app: &mut ExcelApp, cx: &mut Cx, margins: Margins| {
            change(app, t("Margins", "Marges"), cx, |s| s.margins = margins);
        };
        match self {
            Cmd::Print => print::open(app, window, cx),
            // The launcher of the ribbon group opens on Page; the menu
            // entry comes back to the tab the dialog was left on.
            Cmd::Setup => setup::open(app, setup::TAB_PAGE, window, cx),
            Cmd::PageSetupMenu => {
                let tab = app.ext::<PageUi>().map_or(setup::TAB_PAGE, |ui| ui.setup_tab);
                setup::open(app, tab, window, cx)
            }
            Cmd::SetupSheet | Cmd::PrintTitles => setup::open(app, setup::TAB_SHEET, window, cx),
            Cmd::CustomMargins => setup::open(app, setup::TAB_MARGINS, window, cx),
            Cmd::MoreSizes => setup::open(app, setup::TAB_PAGE, window, cx),
            Cmd::Margins => margins_gallery(app, window, cx),
            Cmd::MarginsNormal => margins(app, cx, Margins::NORMAL),
            Cmd::MarginsWide => margins(app, cx, Margins::WIDE),
            Cmd::MarginsNarrow => margins(app, cx, Margins::NARROW),
            Cmd::MarginsLast => {
                if let Some(last) = app.ext::<PageUi>().and_then(|ui| ui.last_custom_margins) {
                    margins(app, cx, last);
                }
            }
            Cmd::Orientation => {
                let entries = list(app, &[Cmd::Portrait, Cmd::Landscape]);
                app.open_command_menu(entries, 200.0, window, cx);
            }
            Cmd::Portrait | Cmd::Landscape => {
                let orientation = if self == Cmd::Portrait {
                    Orientation::Portrait
                } else {
                    Orientation::Landscape
                };
                change(app, t("Orientation", "Orientation"), cx, |s| {
                    s.orientation = orientation
                });
            }
            Cmd::Size => size_gallery(app, window, cx),
            Cmd::PrintArea => {
                let mut entries = list(app, &[Cmd::SetPrintArea, Cmd::ClearPrintArea]);
                // Offered once there is an area to add to.
                if Cmd::AddToPrintArea.enabled(app) {
                    entries.push(MenuEntry::command(Cmd::AddToPrintArea, app));
                }
                app.open_command_menu(entries, 250.0, window, cx);
            }
            Cmd::SetPrintArea => set_print_area(app, cx),
            Cmd::ClearPrintArea | Cmd::ResetPrintArea => {
                if app.wb.clear_print_area(app.sheet) {
                    app.commit(
                        crate::i18n::t_ctx("undo", "Clear Print Area", "Annuler la zone d'impression"),
                        cx,
                    );
                }
            }
            Cmd::AddToPrintArea => {
                let range = app.sel().current();
                if app.wb.add_to_print_area(app.sheet, range) {
                    show_breaks(app, app.sheet);
                    app.commit(Cmd::AddToPrintArea.label(), cx);
                }
            }
            Cmd::Breaks => {
                let entries = list(app, &[Cmd::InsertBreak, Cmd::RemoveBreak, Cmd::ResetBreaks]);
                app.open_command_menu(entries, 260.0, window, cx);
            }
            Cmd::InsertBreak => {
                let (row, col) = breaks_for_selection(app.sel().current());
                if app.wb.insert_page_break(app.sheet, row, col) {
                    show_breaks(app, app.sheet);
                    app.commit(self.label(), cx);
                }
            }
            Cmd::RemoveBreak => {
                let sel = app.sel().current();
                if app.wb.remove_page_break(app.sheet, Some(sel.r0), Some(sel.c0)) {
                    app.commit(self.label(), cx);
                }
            }
            Cmd::ResetBreaks => {
                if app.wb.reset_page_breaks(app.sheet) {
                    app.commit(self.label(), cx);
                }
            }
            Cmd::FitWidth | Cmd::FitHeight => {
                let entries = fit_menu(app, self == Cmd::FitWidth);
                app.open_command_menu(entries, 190.0, window, cx);
            }
            Cmd::Scale => {
                let entries = scale_menu(app);
                app.open_command_menu(entries, 290.0, window, cx);
            }
            Cmd::PrintGridlines => {
                change(app, t("Print Gridlines", "Imprimer le quadrillage"), cx, |s| {
                    s.gridlines = !s.gridlines
                });
            }
            Cmd::PrintHeadings => {
                change(app, t("Print Headings", "Imprimer les en-têtes"), cx, |s| {
                    s.headings = !s.headings
                });
            }
            Cmd::Ruler => {
                let ui = app.ext_mut::<PageUi>();
                ui.ruler_hidden = !ui.ruler_hidden;
                cx.notify();
            }
            Cmd::HeaderFooter => views::insert_header_footer(app, window, cx),
            Cmd::HfHeader | Cmd::HfFooter => {
                let entries = views::preset_menu(app, self == Cmd::HfFooter);
                app.open_command_menu(entries, 360.0, window, cx);
            }
            Cmd::HfPageNumber => views::insert_field(app, "&P", window, cx),
            Cmd::HfPages => views::insert_field(app, "&N", window, cx),
            Cmd::HfDate => views::insert_field(app, "&D", window, cx),
            Cmd::HfTime => views::insert_field(app, "&T", window, cx),
            Cmd::HfPath => views::insert_field(app, "&Z&F", window, cx),
            Cmd::HfFile => views::insert_field(app, "&F", window, cx),
            Cmd::HfSheet => views::insert_field(app, "&A", window, cx),
            Cmd::HfPicture => views::insert_picture(app, window, cx),
            Cmd::HfFormatPicture => views::format_picture(app, window, cx),
            Cmd::HfGoHeader => views::go_to(app, false, window, cx),
            Cmd::HfGoFooter => views::go_to(app, true, window, cx),
            Cmd::HfDifferentFirst => views::toggle_option(app, cx, |hf| hf.different_first = !hf.different_first),
            Cmd::HfDifferentOddEven => {
                views::toggle_option(app, cx, |hf| hf.different_odd_even = !hf.different_odd_even)
            }
            Cmd::HfScaleWithDoc => views::toggle_option(app, cx, |hf| hf.scale_with_doc = !hf.scale_with_doc),
            Cmd::HfAlignMargins => views::toggle_option(app, cx, |hf| hf.align_with_margins = !hf.align_with_margins),
            // Greyed out (see `enabled`); custom views are not part of
            // page setup and have no model.
            Cmd::Background | Cmd::CustomViews => app.not_implemented(self.label(), cx),
        }
    }
}

/// The zoom of the Page Break Preview is its own (60 % at first), as in
/// Excel: swap it with the zoom of the other views when the sheet shown
/// goes into or out of the preview.
fn sync_zoom(app: &mut ExcelApp, cx: &mut Cx) {
    let preview = app.view_mode() == ViewMode::PageBreakPreview;
    let current = app.zoom;
    let ui = app.ext_mut::<PageUi>();
    if preview == ui.zoom_is_preview {
        return;
    }
    ui.zoom_is_preview = preview;
    let target = if preview {
        ui.normal_zoom = current;
        if ui.preview_zoom > 0.0 { ui.preview_zoom } else { 0.6 }
    } else {
        ui.preview_zoom = current;
        if ui.normal_zoom > 0.0 { ui.normal_zoom } else { 1.0 }
    };
    app.set_zoom(target, cx);
}

impl Feature for Page {
    fn name(&self) -> &'static str {
        "page"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        if views::editing_header_footer(app) {
            vec![ContextTab {
                id: HF_TAB,
                title: t("Header & Footer", "En-tête et pied de page"),
            }]
        } else {
            Vec::new()
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        views::header_footer_tab(app, cx)
    }

    fn paint(&self, p: &mut Painter) {
        views::paint(p);
    }

    fn adjust_layout(&self, app: &ExcelApp, layout: &mut GridLayout) {
        views::adjust_layout(app, layout);
    }

    fn overlay_elements(
        &self,
        app: &mut ExcelApp,
        layout: &GridLayout,
        _window: &mut Window,
        _cx: &mut Cx,
    ) -> Vec<AnyElement> {
        views::overlay_elements(app, layout)
    }

    fn zone_mouse_down(
        &self,
        app: &mut ExcelApp,
        zone: &Zone,
        ev: &ZoneEvent,
        window: &mut Window,
        cx: &mut Cx,
    ) -> ZoneResponse {
        views::zone_mouse_down(app, zone, ev, window, cx)
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        views::zone_drag(app, zone, ev, cx);
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        views::zone_mouse_up(app, zone, ev, cx);
    }

    /// Another sheet may be in another view, with another zoom.
    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        if app.ext::<PageUi>().is_some() {
            sync_zoom(app, cx);
        }
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        // Page Break Preview adds its commands to the menu of cells.
        if app.view_mode() != ViewMode::PageBreakPreview || !matches!(target, MenuTarget::Cell(..)) {
            return;
        }
        let mut commands = vec![Cmd::InsertBreak];
        if Cmd::RemoveBreak.enabled(app) {
            commands.push(Cmd::RemoveBreak);
        }
        commands.push(Cmd::ResetBreaks);
        commands.push(Cmd::SetPrintArea);
        if Cmd::AddToPrintArea.enabled(app) {
            commands.push(Cmd::AddToPrintArea);
        }
        commands.extend([Cmd::ResetPrintArea, Cmd::PageSetupMenu]);
        for cmd in commands {
            let entry = MenuEntry::command(cmd, app);
            // In this menu the print area commands are spelled out.
            let entry = match cmd {
                Cmd::SetPrintArea => MenuEntry {
                    label: crate::i18n::t_ctx("undo", "Set Print Area", "Définir la zone d'impression").into(),
                    ..entry
                },
                _ => entry,
            };
            menu.add(MenuSection::Extra, entry);
        }
    }

    fn status_items(&self, app: &ExcelApp) -> Vec<StatusItem> {
        views::status_items(app)
    }

    fn view_mode_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        let mode = app.view_mode();
        let name = app.wb.sheets[app.sheet].name.clone();
        // As last painted, in the view that is being left.
        let corner = app
            .layout
            .as_ref()
            .map(|l| (l.scrolled_rows().start, l.scrolled_cols().start));
        let before = app
            .ext_mut::<PageUi>()
            .last_mode
            .get(&name)
            .copied()
            .unwrap_or_default();
        views::drop_header_footer_edit(app, cx);
        let sheet = &app.wb.sheets[app.sheet];
        if mode == ViewMode::PageLayout && (sheet.frozen_rows > 0 || sheet.frozen_cols > 0) {
            // The view cannot show frozen panes: stay where we were until
            // the user agrees to lose them.
            app.views[app.sheet].mode = before;
            app.alert(
                Alert::new(
                    "OpenSheet",
                    t(
                        "Page Layout View is not compatible with Freeze Panes.  If you continue, the panes on this sheet will be unfrozen. Would you like to continue?",
                        "Le mode d'affichage Mise en page n'est pas compatible avec la fonction Figer les volets. Si vous continuez, les volets de cette feuille seront non figés. Voulez-vous continuer ?",
                    ),
                )
                .warning()
                .button(t("OK", "OK"), |app, _, cx| {
                    app.wb.set_frozen(app.sheet, 0, 0);
                    app.commit(t("Unfreeze Panes", "Libérer les volets"), cx);
                    app.set_view_mode(ViewMode::PageLayout, cx);
                })
                .close_button(t("Cancel", "Annuler")),
                cx,
            );
            return;
        }
        // Back in Normal view the page breaks stay on show, as in Excel.
        if mode == ViewMode::Normal {
            show_breaks(app, app.sheet);
        }
        app.ext_mut::<PageUi>().last_mode.insert(name, mode);
        sync_zoom(app, cx);
        // The cells at the top-left corner of the window stay there: the
        // same scroll offsets mean other cells once the pages are spread
        // on the desk (or gathered again).
        match (corner, app.current_layout()) {
            (Some((row, col)), Some(layout)) => {
                let offset = |axis: &Axis, index: u32, frozen: f32| match index {
                    0 => 0.0,
                    _ => (axis.end(index - 1) - frozen as f64).max(0.0),
                };
                let view = &mut app.views[app.sheet];
                view.scroll_y = offset(&layout.rows, row, layout.frozen_h).min(layout.max_scroll_y());
                view.scroll_x = offset(&layout.cols, col, layout.frozen_w).min(layout.max_scroll_x());
            }
            _ => {
                let (row, col) = app.active();
                app.ensure_visible(row, col);
            }
        }
    }
}

/// Where the grid may draw its gridlines: `None` everywhere (the usual
/// case), or only inside these ranges (Page Break Preview shows none on
/// the grey that does not print).
pub(crate) fn gridline_areas(app: &ExcelApp) -> Option<Vec<Rect>> {
    views::gridline_areas(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_follow_the_locale() {
        let (en, fr) = (Locale::en(), Locale::fr());
        assert!(!metric(en) && metric(fr));
        assert_eq!(length_text(0.75, en), "0.75\"");
        assert_eq!(length_text(0.7, en), "0.7\"");
        assert_eq!(length_text(0.75, fr), "1,91 cm");
        assert_eq!(length_text(1.0, fr), "2,54 cm");
        assert!((from_unit(to_unit(0.3, fr), fr) - 0.3).abs() < 1e-12);
        assert_eq!(to_unit(2.0, en), 2.0);
    }
}
