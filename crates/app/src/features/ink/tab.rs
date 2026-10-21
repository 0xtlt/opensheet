//! The Draw tab: the tools, the gallery of pens with the menu of each
//! pen (thickness, colour, delete), Ink to Shape and Ink Replay.

use super::{Cmd, HIGHLIGHTER_COLORS, InkUi, PEN_COLORS, Pen, Tool, pens, store_pens, tools};
use crate::app::{ExcelApp, Popup, PopupKind};
use crate::features::{Command, Cx};
use crate::i18n::t;
use crate::ribbon::{group, row};
use crate::theme::{self, c, ca};
use crate::view::{icon, tooltip};
use crate::widgets;
use gpui::{AnyElement, AnyView, Context, WeakEntity, Window, div, point, prelude::*, px};
use opensheet_engine::ink::PenKind;

/// The ribbon controls of the pens: a drop-down hangs from a control by
/// its name, and names are fixed. The gallery holds that many pens.
const PEN_IDS: [&str; 12] = [
    "ink-pen-0",
    "ink-pen-1",
    "ink-pen-2",
    "ink-pen-3",
    "ink-pen-4",
    "ink-pen-5",
    "ink-pen-6",
    "ink-pen-7",
    "ink-pen-8",
    "ink-pen-9",
    "ink-pen-10",
    "ink-pen-11",
];

/// How many pens the gallery can hold.
pub(super) const MAX_PENS: usize = PEN_IDS.len();

pub(super) fn kind_name(kind: PenKind) -> &'static str {
    match kind {
        PenKind::Pen => Cmd::AddPenPen.label(),
        PenKind::Pencil => Cmd::AddPencil.label(),
        PenKind::Highlighter => Cmd::AddHighlighter.label(),
    }
}

/// "Pen: 0.5 mm", the tip of a pen of the gallery.
fn pen_tip(pen: &Pen) -> String {
    let mm = widgets::number_text(pen.mm as f64, crate::i18n::lang().locale().decimal_sep);
    crate::i18n::labelled(kind_name(pen.kind), format!("{mm} mm"))
}

/// A pen as the gallery shows it: its body standing over its tip, in its
/// colour, and a stroke of its thickness.
fn pen_picture(pen: Pen) -> impl IntoElement {
    widgets::preview(26.0, 46.0, move |s| {
        let ink = ca(pen.color, 1.0);
        let body = c(0x5A5A5A);
        let mid = s.width / 2.0;
        match pen.kind {
            PenKind::Pen => {
                s.rect(mid - 5.0, 0.0, mid + 5.0, 24.0, body);
                s.rect(mid - 5.0, 20.0, mid + 5.0, 24.0, ink);
                // The nib narrows to a point.
                for (i, half) in [4.0, 3.0, 2.0, 1.0].into_iter().enumerate() {
                    let y = 24.0 + i as f32 * 2.0;
                    s.rect(mid - half, y, mid + half, y + 2.0, ink);
                }
            }
            PenKind::Pencil => {
                s.rect(mid - 4.5, 0.0, mid + 4.5, 22.0, ink);
                s.rect(mid - 4.5, 0.0, mid + 4.5, 3.0, c(0xE8A0A0));
                for (i, half) in [4.0, 3.0, 2.0].into_iter().enumerate() {
                    let y = 22.0 + i as f32 * 2.5;
                    s.rect(mid - half, y, mid + half, y + 2.5, c(0xE9CFA6));
                }
                s.rect(mid - 1.0, 29.5, mid + 1.0, 32.0, c(0x303030));
            }
            PenKind::Highlighter => {
                s.rect(mid - 7.0, 0.0, mid + 7.0, 22.0, body);
                s.rect(mid - 5.0, 22.0, mid + 5.0, 26.0, c(0x7A7A7A));
                s.rect(mid - 4.0, 26.0, mid + 4.0, 32.0, ink);
            }
        }
        // The line it draws, never thicker than the room under the pen.
        let width = (pen.width() * 0.8).clamp(1.0, 7.0);
        let alpha = if pen.kind == PenKind::Highlighter {
            0.6
        } else {
            1.0
        };
        s.line(
            (3.0, 40.0),
            (s.width - 3.0, 40.0),
            width,
            ca(pen.color, alpha),
        );
    })
}

/// One pen of the gallery: a click takes it in hand; a click on the pen
/// in hand opens its menu.
fn pen_button(app: &ExcelApp, index: usize, pen: Pen, cx: &mut Cx) -> AnyElement {
    let id = PEN_IDS[index];
    let in_hand = app
        .ext::<InkUi>()
        .is_some_and(|ui| ui.tool == Tool::Draw(index));
    div()
        .id(id)
        .relative()
        .h(px(68.))
        .w(px(34.))
        .flex()
        .flex_col()
        .items_center()
        .pt(px(6.))
        .gap(px(2.))
        .rounded(px(4.))
        .when(in_hand, |d| d.bg(c(theme::TOGGLED)))
        .hover(|s| s.bg(c(theme::HOVER)))
        .active(|s| s.bg(c(theme::PRESSED)))
        .child(pen_picture(pen))
        .child(icon(
            "chevron-down",
            8.0,
            if in_hand {
                theme::TEXT_MUTED
            } else {
                theme::PANEL_BG
            },
        ))
        .child(app.anchor(id))
        .tooltip(tooltip(pen_tip(&pen)))
        .on_click(cx.listener(move |app, _, window, cx| pen_clicked(app, index, window, cx)))
        .into_any_element()
}

pub(super) fn pen_clicked(app: &mut ExcelApp, index: usize, window: &mut Window, cx: &mut Cx) {
    let in_hand = app
        .ext::<InkUi>()
        .is_some_and(|ui| ui.tool == Tool::Draw(index));
    if in_hand {
        open_pen_menu(app, index, window, cx);
    } else {
        // The press of this click closed whatever menu was open.
        app.popup_dismissed = None;
        tools::take(app, Tool::Draw(index), window, cx);
    }
}

/// The groups of the Draw tab.
pub fn draw_tab(app: &ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let all = pens();
    let mut gallery: Vec<AnyElement> = all
        .iter()
        .take(MAX_PENS)
        .enumerate()
        .map(|(index, pen)| pen_button(app, index, *pen, cx))
        .collect();
    if all.len() < MAX_PENS {
        gallery.push(app.cmd_large("ink-add-pen", Cmd::AddPen, true, cx));
    }
    vec![
        group(
            t("Drawing Tools", "Outils de dessin"),
            row(vec![
                app.cmd_large("ink-select", Cmd::Select, false, cx),
                app.cmd_large("ink-lasso", Cmd::Lasso, false, cx),
                app.cmd_large("ink-eraser", Cmd::Eraser, false, cx),
            ]),
        )
        .into_any_element(),
        group(t("Pens", "Stylets"), row(gallery)).into_any_element(),
        group(
            t("Convert", "Convertir"),
            row(vec![app.cmd_large(
                "ink-to-shape",
                Cmd::InkToShape,
                false,
                cx,
            )]),
        )
        .into_any_element(),
        group(
            t("Replay", "Relire"),
            row(vec![app.cmd_large("ink-replay", Cmd::Replay, false, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

// ------------------------------------------------------------- menu of a pen

/// Change a pen of the gallery.
pub(super) fn change_pen(index: usize, change: impl FnOnce(&mut Pen)) {
    let mut all = pens();
    if let Some(pen) = all.get_mut(index) {
        change(pen);
        store_pens(all);
    }
}

/// Take a pen out of the gallery. The last one stays: Excel's gallery is
/// never empty either.
pub(super) fn delete_pen(app: &mut ExcelApp, index: usize, cx: &mut Cx) {
    let mut all = pens();
    if index >= all.len() || all.len() < 2 {
        return;
    }
    all.remove(index);
    store_pens(all);
    // The pens after it moved up; the one in hand is put down if it is
    // the one that went.
    let ui = app.ext_mut::<InkUi>();
    ui.tool = match ui.tool {
        Tool::Draw(held) if held == index => Tool::Select,
        Tool::Draw(held) if held > index => Tool::Draw(held - 1),
        other => other,
    };
    ui.drag = None;
    cx.notify();
}

/// The drop-down of a pen: thickness, colour, delete.
struct PenMenu {
    app: WeakEntity<ExcelApp>,
    index: usize,
}

fn open_pen_menu(app: &mut ExcelApp, index: usize, window: &mut Window, cx: &mut Cx) {
    let anchor = PEN_IDS[index];
    // A click on the pen while its menu is open (or just closed by the
    // press of this click) closes it.
    let open = app
        .popup
        .as_ref()
        .is_some_and(|popup| popup.anchor == Some(anchor));
    let just_closed = app.popup_dismissed.take() == Some(anchor);
    if open || just_closed {
        app.popup = None;
        cx.notify();
        return;
    }
    let position = app
        .anchors
        .borrow()
        .get(anchor)
        .map(|bounds| {
            point(
                bounds.origin.x,
                bounds.origin.y + bounds.size.height + px(2.),
            )
        })
        .unwrap_or_else(|| window.mouse_position());
    let weak = cx.weak_entity();
    let view: AnyView = cx.new(|_| PenMenu { app: weak, index }).into();
    app.popup = Some(
        Popup::new(PopupKind::View(view), position, 0.0)
            .anchored(anchor)
            .max_height(600.0),
    );
    cx.notify();
}

impl PenMenu {
    /// Redraw the ribbon after a pen changed.
    fn refresh(&self, cx: &mut Context<Self>) {
        self.app.update(cx, |_, cx| cx.notify()).ok();
        cx.notify();
    }
}

impl Render for PenMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let index = self.index;
        let all = pens();
        let Some(pen) = all.get(index).copied() else {
            return div();
        };
        let heading = |text: &'static str| {
            div()
                .text_size(px(11.))
                .text_color(c(theme::TEXT_MUTED))
                .pt(px(2.))
                .child(text)
        };
        let thicknesses = Pen::thicknesses(pen.kind)
            .into_iter()
            .enumerate()
            .map(|(k, mm)| {
                // A dot as wide as the line, within the button.
                let shown = (mm * 96.0 / 25.4).clamp(1.5, 22.0);
                div()
                    .id(("ink-thickness", k))
                    .size(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .when((pen.mm - mm).abs() < 0.001, |d| d.bg(c(theme::TOGGLED)))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(
                        div()
                            .size(px(shown))
                            .rounded(px(shown / 2.0))
                            .bg(c(pen.color))
                            .when(pen.color == 0xFFFFFF, |d| {
                                d.border_1().border_color(c(theme::BORDER_STRONG))
                            }),
                    )
                    .tooltip(tooltip(format!(
                        "{} mm",
                        widgets::number_text(mm as f64, crate::i18n::lang().locale().decimal_sep)
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        change_pen(index, |pen| pen.mm = mm);
                        this.refresh(cx);
                    }))
            });
        let palette: &[u32] = if pen.kind == PenKind::Highlighter {
            &HIGHLIGHTER_COLORS
        } else {
            &PEN_COLORS
        };
        let swatches = palette.iter().copied().enumerate().map(|(k, color)| {
            div()
                .id(("ink-color", k))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(12.))
                .when(pen.color == color, |d| {
                    d.border_2().border_color(c(theme::ACCENT))
                })
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(
                    div()
                        .size(px(16.))
                        .rounded(px(8.))
                        .bg(c(color))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    change_pen(index, |pen| pen.color = color);
                    this.refresh(cx);
                }))
        });
        let last = all.len() < 2;
        let app = self.app.clone();
        div()
            .p(px(8.))
            .w(px(214.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.5))
            .child(heading(t("Thickness", "Épaisseur")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .children(thicknesses),
            )
            .child(heading(t("Color", "Couleur")))
            .child(div().flex().flex_wrap().gap(px(1.)).children(swatches))
            .child(div().h(px(1.)).my(px(3.)).bg(c(theme::BORDER)))
            .child(
                div()
                    .id("ink-delete-pen")
                    .h(px(24.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .when(last, |d| d.text_color(c(theme::TEXT_DISABLED)))
                    .when(!last, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                    .child(t("Delete Pen", "Supprimer le stylo"))
                    .on_click(move |_, _, cx| {
                        if last {
                            return;
                        }
                        app.update(cx, |app, cx| {
                            app.popup = None;
                            delete_pen(app, index, cx);
                        })
                        .ok();
                    }),
            )
    }
}
