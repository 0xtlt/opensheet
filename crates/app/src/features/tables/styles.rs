//! The gallery of table styles: the drop-down of "Format as Table" and of
//! the Table Design tab, the row of styles on that tab, and the
//! thumbnails, drawn from the styles themselves (`TableStyle::patch`).

use super::{Cmd, TablesUi, active_table, dialogs, set_style};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::i18n::{t, t_ctx};
use crate::theme::{self, c};
use crate::widgets::{Sketch, icon, preview, tooltip};
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, MouseDownEvent, Pixels, Stateful,
    WeakEntity, Window, div, point, prelude::*, px,
};
use opensheet_engine::style::Border;
use opensheet_engine::table::{TableLook, TableStyle, builtin_table_styles};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A thumbnail shows a table of this many columns and rows.
const COLS: u32 = 5;
const ROWS: u32 = 5;
const THUMB_W: f32 = 55.0;
const THUMB_H: f32 = 35.0;
/// A thumbnail with its frame.
const ITEM_W: f32 = THUMB_W + 8.0;
const ITEM_H: f32 = THUMB_H + 8.0;
/// Thumbnails per row of the gallery, as in Excel.
const PER_ROW: usize = 7;
/// Width of the button that opens the gallery from the row of styles.
const MORE_W: f32 = 16.0;
/// The ribbon control the gallery of the Table Design tab hangs from. Its
/// bounds also tell how many thumbnails fit on the tab.
const STRIP: &str = "td-styles";
/// The button at the end of that row, which opens the gallery.
const MORE: &str = "td-styles-more";

/// How one cell of a thumbnail is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Swatch {
    fill: Option<u32>,
    /// Colour of the dash standing for the text of the cell.
    ink: u32,
    bold: bool,
    top: Option<u32>,
    bottom: Option<u32>,
    left: Option<u32>,
    right: Option<u32>,
}

/// The options of a table, on the few cells of a thumbnail.
fn thumb_look(options: TableLook) -> TableLook {
    TableLook {
        rows: ROWS,
        cols: COLS,
        ..options
    }
}

/// What the thumbnails show when there is no table to take the options
/// from: those of a new table.
fn new_table_look() -> TableLook {
    TableLook {
        rows: ROWS,
        cols: COLS,
        header_row: true,
        totals_row: false,
        banded_rows: true,
        banded_cols: false,
        first_col: false,
        last_col: false,
    }
}

/// The cells of the thumbnail of a style (`None`: no style), row after
/// row.
fn swatches(style: Option<&TableStyle>, look: &TableLook) -> Vec<Swatch> {
    let edge = |border: Option<Border>| {
        border
            .filter(|border| !border.is_none())
            .map(|border| border.color.map_or(0x000000, |color| color.value()))
    };
    let document = theme::document_theme();
    let mut cells = Vec::with_capacity((ROWS * COLS) as usize);
    for row in 0..ROWS {
        // Bands are counted from the first row of data.
        let band = row.saturating_sub(look.header_row as u32);
        for col in 0..COLS {
            // The style in the colours of the theme of the workbook.
            let mut patch = style
                .map(|style| style.patch(look, row, col, band))
                .unwrap_or_default();
            document.resolve_patch(&mut patch);
            cells.push(Swatch {
                fill: patch.fill.flatten().map(|color| color.value()),
                ink: patch
                    .font_color
                    .flatten()
                    .map_or(0x000000, |color| color.value()),
                bold: patch.bold == Some(true),
                top: edge(patch.border_top),
                bottom: edge(patch.border_bottom),
                left: edge(patch.border_left),
                right: edge(patch.border_right),
            });
        }
    }
    cells
}

/// Draw the thumbnail of a style in a preview box.
fn draw(s: &mut Sketch, style: Option<&TableStyle>, look: &TableLook) {
    let cells = swatches(style, look);
    let at = |row: u32, col: u32| cells[(row * COLS + col) as usize];
    let (cell_w, cell_h) = (s.width / COLS as f32, s.height / ROWS as f32);
    let corners = |row: u32, col: u32| {
        (
            (col as f32 * cell_w).round(),
            (row as f32 * cell_h).round(),
            ((col + 1) as f32 * cell_w).round(),
            ((row + 1) as f32 * cell_h).round(),
        )
    };
    s.rect(0.0, 0.0, s.width, s.height, c(0xFFFFFF));
    for row in 0..ROWS {
        for col in 0..COLS {
            let cell = at(row, col);
            let (x0, y0, x1, y1) = corners(row, col);
            if let Some(fill) = cell.fill {
                s.rect(x0, y0, x1, y1, c(fill));
            }
            // A dash stands for the text of the cell.
            let middle = ((y0 + y1) / 2.0).floor();
            let weight = if cell.bold { 2.0 } else { 1.0 };
            s.rect(x0 + 3.0, middle, x1 - 3.0, middle + weight, c(cell.ink));
        }
    }
    // The borders go over the fills: one line per edge, whichever of the
    // two cells it separates asks for it.
    for row in 0..ROWS {
        for col in 0..COLS {
            let cell = at(row, col);
            let (x0, y0, x1, y1) = corners(row, col);
            let above = row.checked_sub(1).and_then(|row| at(row, col).bottom);
            if let Some(color) = cell.top.or(above) {
                s.rect(x0, y0, x1, y0 + 1.0, c(color));
            }
            let before = col.checked_sub(1).and_then(|col| at(row, col).right);
            if let Some(color) = cell.left.or(before) {
                s.rect(x0, y0, x0 + 1.0, y1, c(color));
            }
            if let Some(color) = cell.bottom.filter(|_| row + 1 == ROWS) {
                s.rect(x0, y1 - 1.0, x1, y1, c(color));
            }
            if let Some(color) = cell.right.filter(|_| col + 1 == COLS) {
                s.rect(x1 - 1.0, y0, x1, y1, c(color));
            }
        }
    }
}

/// The three parts of the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Light,
    Medium,
    Dark,
}

impl Family {
    const ALL: [Family; 3] = [Family::Light, Family::Medium, Family::Dark];

    /// The family of a style ("TableStyleMedium2") and its number in it.
    fn of(name: &str) -> Option<(Family, &str)> {
        [
            ("TableStyleLight", Family::Light),
            ("TableStyleMedium", Family::Medium),
            ("TableStyleDark", Family::Dark),
        ]
        .into_iter()
        .find_map(|(prefix, family)| Some((family, name.strip_prefix(prefix)?)))
    }

    /// Heading of the family in the gallery.
    fn heading(self) -> &'static str {
        match self {
            Family::Light => t("Light", "Clair"),
            Family::Medium => t("Medium", "Moyen"),
            Family::Dark => t("Dark", "Foncé"),
        }
    }

    fn style_title(self) -> &'static str {
        match self {
            Family::Light => t("Table Style Light", "Style de tableau clair"),
            Family::Medium => t("Table Style Medium", "Style de tableau moyen"),
            Family::Dark => t("Table Style Dark", "Style de tableau foncé"),
        }
    }
}

/// The name of a style as the gallery tells it: "Table Style Medium 2".
fn style_title(name: Option<&str>) -> String {
    match name.map(|name| (name, Family::of(name))) {
        None => t_ctx("table style", "None", "Aucun").to_string(),
        Some((_, Some((family, number)))) => format!("{} {number}", family.style_title()),
        Some((name, None)) => name.to_string(),
    }
}

/// The styles of a family in the order of the gallery. "No style" comes
/// first among the light ones.
fn family_styles(family: Family) -> Vec<Option<&'static TableStyle>> {
    let none = (family == Family::Light).then_some(None);
    let styles = builtin_table_styles()
        .iter()
        .filter(move |style| Family::of(&style.name).is_some_and(|(f, _)| f == family))
        .map(Some);
    none.into_iter().chain(styles).collect()
}

/// The rows of the gallery, each family starting a row of its own.
fn gallery_rows() -> Vec<Vec<Option<&'static TableStyle>>> {
    Family::ALL
        .into_iter()
        .flat_map(|family| {
            family_styles(family)
                .chunks(PER_ROW)
                .map(|row| row.to_vec())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The `fit` styles the Table Design tab shows: those of the gallery row
/// holding `current` (the first row when it is not a style of the
/// gallery), moved along when the row is longer than what fits so that
/// `current` is among them.
fn strip_styles(current: Option<&str>, fit: usize) -> Vec<Option<&'static TableStyle>> {
    let name = |style: &Option<&'static TableStyle>| style.map(|style| style.name.as_str());
    let rows = gallery_rows();
    let (row, at) = rows
        .iter()
        .find_map(|row| Some((row, row.iter().position(|style| name(style) == current)?)))
        .unwrap_or((&rows[0], 0));
    let first = (at + 1).saturating_sub(fit);
    row.iter().skip(first).take(fit).copied().collect()
}

/// A thumbnail to click. `current`: it is the style of the table (ringed).
fn thumb(
    index: usize,
    style: Option<&'static TableStyle>,
    look: TableLook,
    current: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let ring = if current {
        c(theme::ACCENT)
    } else {
        gpui::transparent_black()
    };
    div()
        .id(("table-style", index))
        .flex_none()
        .p(px(3.))
        .border_1()
        .border_color(ring)
        .rounded(px(3.))
        .hover(|s| s.bg(c(theme::HOVER)).border_color(c(theme::TEXT_MUTED)))
        .child(preview(THUMB_W, THUMB_H, move |s: &mut Sketch| {
            draw(s, style, &look)
        }))
        .tooltip(tooltip(style_title(style.map(|style| style.name.as_str()))))
        .on_click(on_click)
}

/// The drop-down gallery.
struct Gallery {
    app: WeakEntity<ExcelApp>,
    /// The table options the thumbnails show.
    look: TableLook,
    /// The style of the table being restyled; `None` when the gallery is
    /// about to make a table.
    current: Option<Option<String>>,
    /// Opened from the Table Design tab: "Clear" is offered.
    design: bool,
    /// Where the button that opened the gallery is in the window.
    button: Option<Bounds<Pixels>>,
    /// Set when that button is pressed while the gallery is open (see
    /// `TablesUi::gallery_put_away`).
    put_away: Rc<Cell<Option<Instant>>>,
}

/// A style was picked in the gallery: the table of the active cell takes
/// it, or the Create Table dialog opens to make a table that will.
fn pick(app: &mut ExcelApp, style: Option<&'static str>, window: &mut Window, cx: &mut Cx) {
    app.close_popup(window, cx);
    if active_table(app).is_some() {
        set_style(app, style, cx);
    } else {
        dialogs::open_create(app, Some(style), window, cx);
    }
}

impl Gallery {
    /// A command under the styles, drawn like a menu entry.
    fn command(&self, cmd: Cmd) -> impl IntoElement + use<> {
        let app = self.app.clone();
        div()
            .id(cmd.label())
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(4.))
            .text_size(px(12.5))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(
                div()
                    .w(px(18.))
                    .flex()
                    .justify_center()
                    .when(!cmd.icon().is_empty(), |d| {
                        d.child(icon(cmd.icon(), 14.0, theme::TEXT_MUTED))
                    }),
            )
            .child(cmd.label())
            .on_click(move |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.close_popup(window, cx);
                    cmd.run(app, window, cx);
                })
                .ok();
            })
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for family in Family::ALL {
            body.push(
                div()
                    .h(px(22.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .bg(c(theme::CHROME_BG))
                    .text_size(px(11.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(c(theme::TEXT_MUTED))
                    .child(family.heading())
                    .into_any_element(),
            );
            let thumbs: Vec<AnyElement> = family_styles(family)
                .into_iter()
                .map(|style| {
                    let name = style.map(|style| style.name.as_str());
                    let current = self
                        .current
                        .as_ref()
                        .is_some_and(|current| current.as_deref() == name);
                    let app = self.app.clone();
                    index += 1;
                    thumb(index, style, self.look, current, move |_, window, cx| {
                        app.update(cx, |app, cx| pick(app, name, window, cx)).ok();
                    })
                    .into_any_element()
                })
                .collect();
            body.push(
                div()
                    .w(px(PER_ROW as f32 * ITEM_W))
                    .flex()
                    .flex_wrap()
                    .children(thumbs)
                    .into_any_element(),
            );
        }
        let (button, put_away) = (self.button, self.put_away.clone());
        div()
            .id("table-style-gallery")
            // Any press outside the gallery closes it. One on its own
            // button is remembered: the click it starts must not open the
            // gallery again.
            .on_mouse_down_out(move |ev: &MouseDownEvent, _, _| {
                if button.is_some_and(|bounds| bounds.contains(&ev.position)) {
                    put_away.set(Some(Instant::now()));
                }
            })
            .w(px(PER_ROW as f32 * ITEM_W + 28.0))
            .max_h(px(640.))
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.))
            .children(body)
            .child(div().h(px(1.)).my(px(2.)).bg(c(theme::BORDER)))
            .child(self.command(Cmd::NewStyle))
            .when(self.design, |d| d.child(self.command(Cmd::ClearStyle)))
    }
}

/// Open the gallery under the control that asked for it. `design`: from
/// the Table Design tab.
pub(super) fn open_gallery(app: &mut ExcelApp, design: bool, window: &mut Window, cx: &mut Cx) {
    let put_away = app.ext_mut::<TablesUi>().gallery_put_away.clone();
    // The press of this very click closed the gallery: its button was
    // pressed to put it away, not to open it again. (A press that old was
    // not followed by its click.)
    if put_away
        .take()
        .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
    {
        return;
    }
    let table = active_table(app);
    let look = table.map_or_else(new_table_look, |table| thumb_look(table.look()));
    let current = table.map(|table| table.style.clone());
    let bounds_of = |id: &'static str| app.anchors.borrow().get(id).copied();
    let anchor = app.command_anchor.and_then(bounds_of);
    // On the Table Design tab the gallery hangs from the row of styles; its
    // button is the one at the end of the row.
    let button = if app.command_anchor == Some(STRIP) {
        bounds_of(MORE)
    } else {
        anchor
    };
    let position = match anchor {
        Some(bounds) => point(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height + px(2.),
        ),
        None => window.mouse_position(),
    };
    let weak = cx.weak_entity();
    let view = cx.new(|_| Gallery {
        app: weak,
        look,
        current,
        design,
        button,
        put_away,
    });
    app.open_popup_view(position, view, cx);
}

/// The Table Styles group of the Table Design tab: as many thumbnails as
/// the tab has room for, around the style of the table, and the button
/// that opens the whole gallery. With room for none, the "Quick Styles"
/// button stands for them, as in Excel.
pub(super) fn strip(app: &mut ExcelApp, cx: &mut Cx) -> AnyElement {
    let Some(table) = active_table(app) else {
        return div().into_any_element();
    };
    let look = thumb_look(table.look());
    let current = table.style.clone();
    // The room is known from the frame before; a first frame assumes a
    // whole row.
    let room = app
        .anchors
        .borrow()
        .get(STRIP)
        .map(|bounds| f32::from(bounds.size.width));
    let fit = room.map_or(PER_ROW, |room| {
        ((room - MORE_W - 8.0) / ITEM_W).floor().max(0.0) as usize
    });
    let content = if fit < 2 {
        app.cmd_large("td-quick-styles", Cmd::Styles, true, cx)
    } else {
        let thumbs: Vec<AnyElement> = strip_styles(current.as_deref(), fit.min(PER_ROW))
            .into_iter()
            .enumerate()
            .map(|(index, style)| {
                let name = style.map(|style| style.name.as_str());
                thumb(
                    index,
                    style,
                    look,
                    name == current.as_deref(),
                    cx.listener(move |this, _, _, cx| set_style(this, name, cx)),
                )
                .into_any_element()
            })
            .collect();
        div()
            .flex()
            .items_center()
            .p(px(2.))
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER))
            .rounded(px(4.))
            .children(thumbs)
            .child(
                div()
                    .id(MORE)
                    .relative()
                    .w(px(MORE_W))
                    .h(px(ITEM_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
                    .tooltip(tooltip(Cmd::Styles.label()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        // The gallery hangs from the row of styles.
                        this.command_anchor = Some(STRIP);
                        Cmd::Styles.run(this, window, cx);
                        this.command_anchor = None;
                    }))
                    .child(app.anchor(MORE)),
            )
            .into_any_element()
    };
    // This box takes the width the other groups leave: it is what `room`
    // measures.
    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .overflow_hidden()
        .flex()
        .items_center()
        .child(content)
        .child(app.anchor(STRIP))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Color;
    use opensheet_engine::table::{DEFAULT_TABLE_STYLE, OFFICE_THEME, table_style, tint};

    fn names(styles: &[Option<&'static TableStyle>]) -> Vec<&'static str> {
        styles
            .iter()
            .map(|style| style.map_or("", |style| style.name.as_str()))
            .collect()
    }

    #[test]
    fn gallery_is_laid_out_like_excels() {
        let light = family_styles(Family::Light);
        assert_eq!(light.len(), 22);
        assert!(light[0].is_none());
        assert_eq!(names(&light)[1], "TableStyleLight1");
        assert_eq!(names(&light)[21], "TableStyleLight21");
        assert_eq!(family_styles(Family::Medium).len(), 28);
        assert_eq!(family_styles(Family::Dark).len(), 11);
        // Every built-in style is in one family.
        let total: usize = Family::ALL
            .into_iter()
            .map(|family| family_styles(family).len())
            .sum();
        assert_eq!(total, builtin_table_styles().len() + 1);
        // Rows of seven, each family starting its own.
        let rows = gallery_rows();
        assert_eq!(
            rows.iter().map(Vec::len).collect::<Vec<_>>(),
            [7, 7, 7, 1, 7, 7, 7, 7, 7, 4]
        );
        assert_eq!(names(&rows[4])[0], "TableStyleMedium1");
        assert_eq!(names(&rows[8])[0], "TableStyleDark1");
    }

    #[test]
    fn titles() {
        assert_eq!(
            style_title(Some("TableStyleMedium2")),
            "Table Style Medium 2"
        );
        assert_eq!(
            style_title(Some("TableStyleLight21")),
            "Table Style Light 21"
        );
        assert_eq!(style_title(Some("TableStyleDark11")), "Table Style Dark 11");
        assert_eq!(style_title(None), "None");
        // A style of a file that is not built in keeps its name.
        assert_eq!(style_title(Some("My Style")), "My Style");
    }

    #[test]
    fn strip_shows_the_row_of_the_current_style() {
        // The default style is the second of the first row of medium ones.
        let strip = strip_styles(Some(DEFAULT_TABLE_STYLE), 7);
        assert_eq!(names(&strip)[0], "TableStyleMedium1");
        assert_eq!(strip.len(), 7);
        // With less room the row starts the same...
        assert_eq!(
            names(&strip_styles(Some(DEFAULT_TABLE_STYLE), 3)),
            [
                "TableStyleMedium1",
                "TableStyleMedium2",
                "TableStyleMedium3"
            ]
        );
        // ...unless the current style would be cut off.
        assert_eq!(
            names(&strip_styles(Some("TableStyleMedium7"), 3)),
            [
                "TableStyleMedium5",
                "TableStyleMedium6",
                "TableStyleMedium7"
            ]
        );
        // No style: the first row, which starts with it.
        assert_eq!(
            names(&strip_styles(None, 4)),
            [
                "",
                "TableStyleLight1",
                "TableStyleLight2",
                "TableStyleLight3"
            ]
        );
        // A style that is not in the gallery: the first row too.
        assert_eq!(
            names(&strip_styles(Some("Custom"), 2)),
            ["", "TableStyleLight1"]
        );
        // The short last row of a family.
        assert_eq!(
            names(&strip_styles(Some("TableStyleLight21"), 7)),
            ["TableStyleLight21"]
        );
    }

    #[test]
    fn thumbnail_follows_the_style_and_the_options() {
        let style = table_style(DEFAULT_TABLE_STYLE).unwrap();
        let look = new_table_look();
        let cells = swatches(Some(style), &look);
        assert_eq!(cells.len(), 25);
        // Medium 2: a header in the first accent colour with white bold
        // text, then bands.
        let accent = tint(Color(OFFICE_THEME[4]), 0.0).value();
        assert_eq!(cells[0].fill, Some(accent));
        assert_eq!(cells[0].ink, 0xFFFFFF);
        assert!(cells[0].bold);
        let (first, second) = (cells[COLS as usize], cells[2 * COLS as usize]);
        assert!(first.fill.is_some());
        assert_ne!(first.fill, second.fill);
        assert_eq!(first.fill, cells[3 * COLS as usize].fill);
        // Without banded rows the data rows look alike; with a totals row
        // the last row differs from them.
        let plain = TableLook {
            banded_rows: false,
            totals_row: true,
            ..look
        };
        let cells = swatches(Some(style), &plain);
        assert_eq!(cells[COLS as usize].fill, cells[2 * COLS as usize].fill);
        assert!(cells[4 * COLS as usize].bold);
        // No style draws nothing but black dashes.
        assert!(
            swatches(None, &look)
                .iter()
                .all(|cell| *cell == Swatch::default())
        );
    }

    #[test]
    fn thumbnail_options_keep_the_thumbnail_size() {
        let look = thumb_look(TableLook {
            rows: 40,
            cols: 12,
            header_row: false,
            totals_row: true,
            banded_rows: false,
            banded_cols: true,
            first_col: true,
            last_col: true,
        });
        assert_eq!((look.rows, look.cols), (ROWS, COLS));
        assert!(
            look.totals_row
                && look.banded_cols
                && look.first_col
                && look.last_col
                && !look.header_row
        );
    }
}
