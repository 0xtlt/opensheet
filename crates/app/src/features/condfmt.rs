//! Conditional formatting: the menu of the Home tab with its galleries,
//! the rule dialogs, the Rules Manager, and what rules draw inside cells
//! (data bars, icons).
//!
//! * `draw` — data bars and icons as vector shapes, for the grid and for
//!   every preview;
//! * `menu` — the drop-down of the ribbon button;
//! * `quick` — the small dialogs of the Highlight Cells and Top/Bottom
//!   menus, with their live preview on the sheet;
//! * `rule_dialog` — New Formatting Rule / Edit Formatting Rule, around
//!   the `draft` of the rule being made;
//! * `manager` — the Conditional Formatting Rules Manager;
//! * `words` — Excel's wording of presets and rules.
//!
//! The grid paints cells with the style `Workbook::display` returns (the
//! fills of colour scales and highlight formats come that way) and calls
//! [`paint_adornment`] for the cells whose rules draw something.

mod draft;
mod draw;
mod manager;
mod menu;
mod quick;
mod rule_dialog;
mod words;

use super::{Command, Cx, Feature, Intent};
use crate::app::ExcelApp;
use crate::grid::{Draw, GridLayout, pt_to_px};
use crate::i18n::t;
use crate::theme::{self, c};
use gpui::{AnyElement, Entity, FontWeight, Window, WindowTextSystem, div, prelude::*, px};
use opensheet_engine::condfmt::{Adornment, CondFormat, Rule};
use opensheet_engine::style::{Border, HAlign, StylePatch, Underline, VAlign};
use opensheet_engine::{Rect, Style, Value};
use std::time::Instant;

pub struct CondFormats;

commands! {
    /// What the user can ask of conditional formatting.
    pub enum Cmd {
        Menu = "cond-format", t("Conditional Formatting", "Mise en forme conditionnelle");
        GreaterThan = "", t("Greater Than…", "Supérieur à…");
        LessThan = "", t("Less Than…", "Inférieur à…");
        Between = "", t("Between…", "Entre…");
        EqualTo = "", t("Equal To…", "Égal à…");
        TextContains = "", t("Text that Contains…", "Texte qui contient…");
        DateOccurring = "", t("A Date Occurring…", "Une date se produisant…");
        DuplicateValues = "", t("Duplicate Values…", "Valeurs en double…");
        Top10Items = "", t("Top 10 Items…", "10 valeurs les plus élevées…");
        Top10Percent = "", t("Top 10 %…", "10 % de valeurs les plus élevées…");
        Bottom10Items = "", t("Bottom 10 Items…", "10 valeurs les moins élevées…");
        Bottom10Percent = "", t("Bottom 10 %…", "10 % de valeurs les moins élevées…");
        AboveAverage = "", t("Above Average…", "Valeurs supérieures à la moyenne…");
        BelowAverage = "", t("Below Average…", "Valeurs inférieures à la moyenne…");
        MoreHighlight = "", t("More Rules…", "Autres règles…");
        MoreTopBottom = "", t("More Rules…", "Autres règles…");
        DataBars = "", t("More Rules…", "Autres règles…");
        ColorScales = "", t("More Rules…", "Autres règles…");
        IconSets = "", t("More Rules…", "Autres règles…");
        NewRule = "condfmt-new", t("New Rule…", "Nouvelle règle…");
        ClearSelected = "", t("Clear Rules from Selected Cells", "Effacer les règles des cellules sélectionnées");
        ClearSheet = "", t("Clear Rules from Entire Sheet", "Effacer les règles de la feuille entière");
        ClearTable = "", t("Clear Rules from This Table", "Effacer les règles de ce tableau");
        ClearPivot = "", t("Clear Rules from This PivotTable", "Effacer les règles de ce tableau croisé dynamique");
        Manage = "condfmt-manage", t("Manage Rules…", "Gérer les règles…");
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
        let (row, col) = app.active();
        match self {
            Cmd::ClearTable => app.wb.table_at(app.sheet, row, col).is_some(),
            Cmd::ClearPivot => app.wb.pivot_at(app.sheet, row, col).is_some(),
            _ => true,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if self == Cmd::Menu {
            return menu::toggle(app, window, cx);
        }
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        // A protected sheet lets conditional formats change with its
        // "Format cells" permission, as Excel does.
        if !app.may_all(Intent::FormatCells, cx) {
            return;
        }
        match self {
            Cmd::Menu => {}
            Cmd::GreaterThan => quick::open(app, quick::Kind::GreaterThan, window, cx),
            Cmd::LessThan => quick::open(app, quick::Kind::LessThan, window, cx),
            Cmd::Between => quick::open(app, quick::Kind::Between, window, cx),
            Cmd::EqualTo => quick::open(app, quick::Kind::EqualTo, window, cx),
            Cmd::TextContains => quick::open(app, quick::Kind::TextContains, window, cx),
            Cmd::DateOccurring => quick::open(app, quick::Kind::DateOccurring, window, cx),
            Cmd::DuplicateValues => quick::open(app, quick::Kind::DuplicateValues, window, cx),
            Cmd::Top10Items => quick::open(app, quick::Kind::TopItems, window, cx),
            Cmd::Top10Percent => quick::open(app, quick::Kind::TopPercent, window, cx),
            Cmd::Bottom10Items => quick::open(app, quick::Kind::BottomItems, window, cx),
            Cmd::Bottom10Percent => quick::open(app, quick::Kind::BottomPercent, window, cx),
            Cmd::AboveAverage => quick::open(app, quick::Kind::AboveAverage, window, cx),
            Cmd::BelowAverage => quick::open(app, quick::Kind::BelowAverage, window, cx),
            Cmd::MoreHighlight => {
                rule_dialog::open_new(app, rule_dialog::Start::Contains, window, cx)
            }
            Cmd::MoreTopBottom => {
                rule_dialog::open_new(app, rule_dialog::Start::Ranked, window, cx)
            }
            Cmd::DataBars => rule_dialog::open_new(app, rule_dialog::Start::DataBar, window, cx),
            Cmd::ColorScales => {
                rule_dialog::open_new(app, rule_dialog::Start::ColorScale, window, cx)
            }
            Cmd::IconSets => rule_dialog::open_new(app, rule_dialog::Start::IconSet, window, cx),
            Cmd::NewRule => rule_dialog::open_new(app, rule_dialog::Start::ColorScale, window, cx),
            Cmd::ClearSelected => {
                let ranges = app.sel().ranges.clone();
                clear_rules(app, Some(ranges), cx);
            }
            Cmd::ClearSheet => clear_rules(app, None, cx),
            Cmd::ClearTable => {
                let (row, col) = app.active();
                if let Some(range) = app
                    .wb
                    .table_at(app.sheet, row, col)
                    .map(|table| table.range)
                {
                    clear_rules(app, Some(vec![range]), cx);
                }
            }
            Cmd::ClearPivot => {
                let (row, col) = app.active();
                let ranges = app
                    .wb
                    .pivot_at(app.sheet, row, col)
                    .map(|pivot| pivot.footprint().collect::<Vec<Rect>>());
                if let Some(ranges) = ranges {
                    clear_rules(app, Some(ranges), cx);
                }
            }
            Cmd::Manage => manager::open(app, window, cx),
        }
    }
}

/// Label of the undo step of whatever changes rules.
fn undo_label() -> &'static str {
    t("Conditional Formatting", "Mise en forme conditionnelle")
}

/// Add a rule on the selected cells, with the highest priority.
fn add_rule(app: &mut ExcelApp, rule: Rule, format: StylePatch, cx: &mut Cx) {
    end_preview(app, cx);
    let sheet = app.sheet;
    let ranges = app.sel().ranges.clone();
    if app
        .wb
        .add_cond_format(sheet, CondFormat::new(ranges, rule).with_format(format))
    {
        app.commit(undo_label(), cx);
    }
}

/// Take the rules off the cells of `ranges`, or off the whole sheet.
fn clear_rules(app: &mut ExcelApp, ranges: Option<Vec<Rect>>, cx: &mut Cx) {
    end_preview(app, cx);
    let sheet = app.sheet;
    let changed = match ranges {
        Some(ranges) => app.wb.clear_cond_formats(sheet, &ranges),
        None => app.wb.clear_all_cond_formats(sheet),
    };
    if changed {
        app.commit(t("Clear Rules", "Effacer les règles"), cx);
    }
}

// ------------------------------------------------------------------ preview

/// What the feature keeps between events.
#[derive(Default)]
struct Ui {
    /// While a rule is being tried on the sheet (a dialog is open, the
    /// pointer rests on a gallery): the sheet and its real rules.
    preview: Option<(usize, Vec<CondFormat>)>,
    /// The open drop-down of the ribbon button.
    menu: Option<Entity<menu::MenuView>>,
    /// When that drop-down was last closed by a press outside it.
    menu_closed: Option<Instant>,
}

/// Show the current sheet as if `rule` had been added on top of its
/// rules; `None` shows the rules as they are. Nothing is recorded: the
/// rules come back with [`end_preview`], which must run before anything
/// that takes an undo snapshot.
fn preview(app: &mut ExcelApp, rule: Option<CondFormat>, cx: &mut Cx) {
    let sheet = app.sheet;
    let state = app.ext_mut::<Ui>();
    if state
        .preview
        .as_ref()
        .is_some_and(|(previewed, _)| *previewed != sheet)
    {
        end_preview(app, cx);
    }
    let original = match &app.ext_mut::<Ui>().preview {
        Some((_, original)) => original.clone(),
        None => {
            if rule.is_none() {
                return;
            }
            let original = app.wb.sheets[sheet].cond_formats.clone();
            app.ext_mut::<Ui>().preview = Some((sheet, original.clone()));
            original
        }
    };
    let mut rules = original;
    if let Some(rule) = rule.filter(|rule| !rule.ranges.is_empty()) {
        rules.insert(0, rule);
    }
    app.wb.sheets[sheet].cond_formats = rules;
    app.wb.caches.cond.invalidate();
    cx.notify();
}

/// Put back the rules a preview replaced.
fn end_preview(app: &mut ExcelApp, cx: &mut Cx) {
    if let Some((sheet, original)) = app.ext_mut::<Ui>().preview.take() {
        if let Some(sh) = app.wb.sheets.get_mut(sheet) {
            sh.cond_formats = original;
        }
        app.wb.caches.cond.invalidate();
        cx.notify();
    }
}

/// A sample of text wearing a format, as the dialogs preview it; "No
/// Format Set" for a format that changes nothing.
fn format_sample(format: &StylePatch, width: f32, height: f32) -> AnyElement {
    let frame = div()
        .w(px(width))
        .h(px(height))
        .flex_none()
        .p(px(2.))
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(theme::BORDER_STRONG));
    if format.is_empty() {
        return frame
            .flex()
            .items_center()
            .justify_center()
            .text_color(c(theme::TEXT_MUTED))
            .child(t("No Format Set", "Aucun format défini"))
            .into_any_element();
    }
    let line = |border: Option<Border>| border.filter(|b| !b.is_none());
    let edges = [
        format.border_top,
        format.border_right,
        format.border_bottom,
        format.border_left,
    ]
    .map(line);
    let line_color = edges
        .iter()
        .flatten()
        .next()
        .map(|b| b.color.map(|color| color.value()).unwrap_or(0x000000));
    let side = |edge: Option<Border>| px(if edge.is_some() { 1.0 } else { 0.0 });
    frame
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .when_some(format.fill.flatten(), |d, fill| d.bg(c(fill.value())))
                .when_some(format.font_color.flatten(), |d, color| {
                    d.text_color(c(color.value()))
                })
                .when(format.bold == Some(true), |d| {
                    d.font_weight(FontWeight::BOLD)
                })
                .when(format.italic == Some(true), |d| d.italic())
                .when(
                    format.underline.is_some_and(|u| u != Underline::None),
                    |d| d.underline(),
                )
                .when(format.strike == Some(true), |d| d.line_through())
                .when_some(line_color, |d, color| {
                    d.border_color(c(color))
                        .border_t(side(edges[0]))
                        .border_r(side(edges[1]))
                        .border_b(side(edges[2]))
                        .border_l(side(edges[3]))
                })
                .child("AaBbCcYyZz"),
        )
        .into_any_element()
}

// ----------------------------------------------------------------- the grid

/// The cell an adornment is drawn in.
// A record the grid fills in; drawing needs only part of it.
#[allow(dead_code)]
pub struct CellBox<'a> {
    pub row: u32,
    pub col: u32,
    /// x0, y0, x1, y1 in grid-element coordinates.
    pub rect: (f32, f32, f32, f32),
    pub value: &'a Value,
    /// The cell's style, conditional formats included.
    pub style: &'a Style,
    /// For positions (`to_window`, `bounds_px`) and the zoom.
    pub layout: &'a GridLayout,
    pub ts: &'a WindowTextSystem,
}

/// What an adornment draws and the room it takes from the text of its
/// cell.
#[derive(Default)]
pub struct AdornmentPaint {
    /// Drawn over the cell's fill, under its text (the data bar).
    pub under: Vec<Draw>,
    /// Drawn with the text (the icon).
    pub over: Vec<Draw>,
    /// Room taken from the left and the right of the text (by the icon).
    pub text_inset: (f32, f32),
}

/// Where the icon of a cell goes: its left edge, its top edge and its
/// side, in the coordinates of `rect`. The icon is as tall as a line of
/// the cell's text (16 pixels for 11 points at 100 %) and sits on that
/// line; it stays at the left of the cell whatever the alignment, unless
/// the value is hidden: an icon alone follows the horizontal alignment.
fn icon_place(
    rect: (f32, f32, f32, f32),
    style: &Style,
    zoom: f32,
    alone: bool,
) -> (f32, f32, f32) {
    let (x0, y0, x1, y1) = rect;
    let (width, height) = (x1 - x0 - 1.0, y1 - y0 - 1.0);
    let font_px = pt_to_px(style.font.size_pt()) * zoom;
    let line = (font_px * 1.22).round();
    let margin = zoom.round().max(1.0);
    let side = (style.font.size_pt() * 16.0 / 11.0 * zoom)
        .round()
        .min(height - 1.0)
        .min(width - 2.0 * margin)
        .max(0.0);
    let x = match style.h_align {
        HAlign::Center | HAlign::CenterAcross if alone => x0 + ((width - side) / 2.0).round(),
        HAlign::Right if alone => x1 - 1.0 - margin - side,
        _ => x0 + margin,
    };
    // The top of the line of text the grid would draw, then the icon
    // centred on it.
    let text_top = match style.v_align {
        VAlign::Top => y0 + 1.0,
        VAlign::Center => y0 + ((y1 - y0 - line) / 2.0).round(),
        VAlign::Bottom => y1 - line - zoom.round().max(1.0),
    };
    let y = (text_top + ((line - side) / 2.0).round())
        .min(y1 - 1.0 - side)
        .max(y0);
    (x, y, side)
}

/// Draw what a conditional format adds inside a cell: `adornment.bar`
/// (fractions of the cell's width, already mirrored and coloured by the
/// engine) and `adornment.icon`. Called by the grid for every visible cell
/// whose `Workbook::display` carries an adornment; the grid itself leaves
/// the value out when `adornment.hide_value` is set.
pub fn paint_adornment(adornment: &Adornment, cell: &CellBox) -> AdornmentPaint {
    let mut paint = AdornmentPaint::default();
    let zoom = cell.layout.zoom;
    let (x0, y0, x1, y1) = cell.rect;
    let origin = cell.layout.to_window(x0, y0);
    let (left, top) = (f32::from(origin.x), f32::from(origin.y));
    if let Some(bar) = &adornment.bar {
        draw::bar(
            bar,
            (left, top, left + (x1 - x0), top + (y1 - y0)),
            zoom,
            &mut paint.under,
        );
    }
    if let Some(icon) = adornment.icon {
        let (x, y, side) = icon_place(cell.rect, cell.style, zoom, adornment.hide_value);
        if side >= 2.0 {
            draw::icon(
                icon.icon(),
                left + (x - x0),
                top + (y - y0),
                side,
                &mut paint.over,
            );
            // The text starts after the icon (the grid adds its own padding).
            paint.text_inset.0 = x - x0 + side;
        }
    }
    paint
}

impl Feature for CondFormats {
    fn name(&self) -> &'static str {
        "condfmt"
    }

    // Nothing is laid over the grid: this hook runs at every frame, which
    // is when a drop-down closed by a press elsewhere is noticed and the
    // rule its gallery was trying on the sheet withdrawn.
    fn overlay_elements(
        &self,
        app: &mut ExcelApp,
        _: &GridLayout,
        _: &mut Window,
        cx: &mut Cx,
    ) -> Vec<AnyElement> {
        menu::notice_closed(app, cx);
        Vec::new()
    }

    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        // A preview belongs to the cells that were selected when it began.
        if !app.modal_open() {
            end_preview(app, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(style: &Style, zoom: f32, alone: bool) -> (f32, f32, f32) {
        // A cell of the default size: 64 × 20 pixels, gridlines included.
        let rect = (100.0, 40.0, 100.0 + 64.0 * zoom, 40.0 + 20.0 * zoom);
        icon_place(rect, style, zoom, alone)
    }

    #[test]
    fn the_icon_of_default_text_is_sixteen_pixels_at_the_left() {
        let (x, y, side) = place(&Style::default(), 1.0, false);
        // As in Excel: one pixel from the gridline at its left, two free
        // above it and one below.
        assert_eq!((x, y, side), (101.0, 42.0, 16.0));
    }

    #[test]
    fn the_icon_follows_the_zoom_and_the_font_size() {
        assert_eq!(place(&Style::default(), 2.0, false).2, 32.0);
        let mut big = Style::default();
        big.font.set_size_pt(22.0);
        // Twice the font would be twice the icon, but the row is not that
        // tall: the icon shrinks to fit.
        assert_eq!(place(&big, 1.0, false).2, 18.0);
    }

    #[test]
    fn an_icon_alone_follows_the_alignment() {
        let mut style = Style::default();
        let left = place(&style, 1.0, true).0;
        style.h_align = HAlign::Center;
        let centre = place(&style, 1.0, true).0;
        style.h_align = HAlign::Right;
        let right = place(&style, 1.0, true).0;
        assert_eq!(left, 101.0);
        assert_eq!(centre, 124.0);
        assert_eq!(right, 146.0);
        // With its value shown, the icon stays at the left.
        assert_eq!(place(&style, 1.0, false).0, 101.0);
    }
}
