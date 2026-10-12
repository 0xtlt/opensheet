//! Every widget in one dialog, for trying the toolkit by hand and as a
//! working example of each piece: `opensheet --widgets`.
//!
//! The dialog can also open a side pane, a drop-down hanging from the
//! active cell and a message box. While its pane is open, the gallery marks
//! the active cell with a clickable corner: the smallest example of
//! painting on the grid and registering a zone (see `features/mod.rs`).

use super::{
    Button, CheckItem, CheckList, Checkbox, ColorButton, ColorGrid, ComboBox, GroupBox, ListBox, NumberField, RadioGroup,
    RefEdit, ScrollArea, Select, Sketch, TabStrip, TextField, Tree, TreeNode, Tri, dialog, input, label, labeled,
    number_input, ok_cancel, preview, separator, ui_font,
};
use crate::app::ExcelApp;
use crate::features::{Cx, Feature, PaintLayer, Painter, StatusItem, Zone, ZoneEvent, ZoneResponse};
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use gpui::{AnyElement, Context, CursorStyle, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::style::Color;

const PANE: &str = "widget-gallery";

/// Open the gallery over the window.
pub fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    app.open_modal(window, cx, |modal, _, cx| Gallery::new(modal, cx));
}

fn names(items: &[&'static str]) -> Vec<SharedString> {
    items.iter().map(|name| (*name).into()).collect()
}

fn sample_tree() -> Vec<TreeNode> {
    let month = |name: &'static str, days: &[u32]| {
        TreeNode::branch(name, days.iter().map(|day| TreeNode::leaf(day.to_string(), day % 2 == 0)).collect())
    };
    let mut year = TreeNode::branch("2026", vec![month("January", &[5, 12, 19]), month("February", &[2, 18])]);
    year.expanded = true;
    vec![year, TreeNode::branch("2025", vec![month("December", &[1, 24, 31])])]
}

fn sample_checks() -> Vec<CheckItem> {
    ["North", "South", "East", "West", "Centre", "Islands", "Overseas", "Online", "Wholesale", "Retail", "Export"]
        .iter()
        .enumerate()
        .map(|(i, name)| CheckItem::new(*name, i % 3 != 1))
        .collect()
}

struct Gallery {
    modal: Modal,
    tab: usize,
    check: bool,
    tri: Tri,
    radio: Option<usize>,
    layout: Option<usize>,
    select: Option<usize>,
    combo: Entity<TextInput>,
    text: Entity<TextInput>,
    number: Entity<TextInput>,
    color: Option<Option<Color>>,
    fill: Option<Option<Color>>,
    list: Option<usize>,
    multi: Vec<usize>,
    checks: Vec<CheckItem>,
    search: Entity<TextInput>,
    tree: Vec<TreeNode>,
    range: Entity<TextInput>,
    cells: Entity<TextInput>,
    activated: SharedString,
}

impl Gallery {
    fn new(modal: Modal, cx: &mut Context<Self>) -> Self {
        let search = input(cx, "");
        search.update(cx, |field, _| field.placeholder = "Search".into());
        Gallery {
            tab: 0,
            check: true,
            tri: Tri::Mixed,
            radio: Some(0),
            layout: None,
            select: Some(1),
            combo: input(cx, "11"),
            text: input(cx, "Table1"),
            number: number_input(cx, Some(2.0), 0.0, 30.0, 1.0),
            color: Some(None),
            fill: None,
            list: Some(2),
            multi: vec![1, 3],
            checks: sample_checks(),
            search,
            tree: sample_tree(),
            range: modal.ref_input(RefStyle::formula(), "", cx),
            cells: modal.ref_input(RefStyle::absolute(), "", cx),
            activated: SharedString::default(),
            modal,
        }
    }

    fn basics(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap(px(24.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        GroupBox::new("Check boxes")
                            .child(Checkbox::new("check", "Wrap text", self.check).on_change(cx.listener(
                                |this, on: &bool, _, cx| {
                                    this.check = *on;
                                    cx.notify();
                                },
                            )))
                            .child(Checkbox::new("tri", "Three states", self.tri).on_change_tri(cx.listener(
                                |this, state: &Tri, _, cx| {
                                    this.tri = *state;
                                    cx.notify();
                                },
                            )))
                            .child(Checkbox::new("off", "Disabled", true).disabled(true)),
                    )
                    .child(
                        GroupBox::new("Radio groups")
                            .child(
                                RadioGroup::new("radio", names(&["Shift cells right", "Shift cells down"]), self.radio)
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        this.radio = Some(*ix);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                RadioGroup::new("layout", names(&["Rows", "Columns"]), self.layout)
                                    .horizontal()
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        this.layout = Some(*ix);
                                        cx.notify();
                                    })),
                            )
                            .child(RadioGroup::new("radio-off", names(&["Disabled"]), None).disabled(true)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(labeled(
                        "Select:",
                        Select::new("select", names(&["None", "Single", "Double", "Single Accounting"]), self.select)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.select = Some(*ix);
                                cx.notify();
                            })),
                    ))
                    .child(labeled("Disabled:", Select::new("select-off", names(&["Automatic"]), Some(0)).disabled(true)))
                    .child(labeled(
                        "Combo box:",
                        ComboBox::new("combo", &self.combo, names(&["8", "9", "10", "11", "12", "14", "16", "18"]))
                            .width(70.0)
                            .on_pick(cx.listener(|_, _: &usize, _, cx| cx.notify())),
                    ))
                    .child(labeled("Text:", TextField::new(&self.text).width(140.0)))
                    .child(labeled("Number:", NumberField::new("number", &self.number)))
                    .child(labeled(
                        "Colour:",
                        ColorButton::new("color", self.color)
                            .none_label("Automatic")
                            .more(&self.modal.app)
                            .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| {
                                this.color = Some(*color);
                                cx.notify();
                            })),
                    ))
                    .child(separator())
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(Button::new("primary", "Default").primary())
                            .child(Button::new("plain", "Button").width(90.0))
                            .child(Button::new("disabled", "Disabled").disabled(true)),
                    ),
            )
            .into_any_element()
    }

    fn lists(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let fonts = names(&[
            "Aptos", "Arial", "Calibri", "Cambria", "Courier New", "Georgia", "Helvetica", "Menlo", "Optima", "Palatino",
            "Tahoma", "Times New Roman", "Verdana",
        ]);
        div()
            .flex()
            .gap(px(12.))
            .child(
                div()
                    .w(px(130.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(label("List box:"))
                    .child(
                        ListBox::new("list", fonts.clone(), self.list)
                            .height(120.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.list = Some(*ix);
                                cx.notify();
                            }))
                            .on_activate(cx.listener(|this, ix: &usize, _, cx| {
                                this.activated = format!("double-clicked item {ix}").into();
                                cx.notify();
                            })),
                    )
                    .child(label("Multiple:"))
                    .child(
                        ListBox::multi("multi", fonts, self.multi.clone())
                            .height(100.0)
                            .on_change_multi(cx.listener(|this, rows: &Vec<usize>, _, cx| {
                                this.multi = rows.clone();
                                cx.notify();
                            })),
                    )
                    .child(label(self.activated.clone()).text_color(c(theme::TEXT_MUTED))),
            )
            .child(
                div().flex().flex_col().gap(px(3.)).child(label("Check-list with search:")).child(
                    CheckList::new("checks", self.checks.clone())
                        .search(&self.search)
                        .width(170.0)
                        .height(180.0)
                        .on_toggle(cx.listener(|this, (ix, on): &(usize, bool), _, cx| {
                            this.checks[*ix].checked = *on;
                            cx.notify();
                        }))
                        .on_toggle_all(cx.listener(|this, (shown, on): &(Vec<usize>, bool), _, cx| {
                            for ix in shown {
                                this.checks[*ix].checked = *on;
                            }
                            cx.notify();
                        })),
                ),
            )
            .child(
                div().flex().flex_col().gap(px(3.)).child(label("Tree:")).child(
                    Tree::new("tree", &self.tree)
                        .width(170.0)
                        .height(208.0)
                        .on_toggle(cx.listener(|this, path: &Vec<usize>, _, cx| {
                            TreeNode::toggle(&mut this.tree, path);
                            cx.notify();
                        }))
                        .on_expand(cx.listener(|this, path: &Vec<usize>, _, cx| {
                            TreeNode::toggle_expanded(&mut this.tree, path);
                            cx.notify();
                        }))
                        .on_select(cx.listener(|this, path: &Vec<usize>, _, cx| {
                            this.activated = format!("node {path:?}").into();
                            cx.notify();
                        })),
                ),
            )
            .into_any_element()
    }

    fn more(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let selection = self
            .modal
            .read_app(cx, |app| app.sel().current().name())
            .unwrap_or_default();
        let fill = self.fill.flatten().map(|color| color.0).unwrap_or(0xFFFFFF);
        div()
            .flex()
            .gap(px(20.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(labeled("Range (pick cells on the sheet):", RefEdit::new(&self.range, &self.modal)))
                    .child(labeled("Cells:", RefEdit::new(&self.cells, &self.modal).width(120.0)))
                    .child(label(format!("Selection of the sheet: {selection}")).text_color(c(theme::TEXT_MUTED)))
                    .child(
                        GroupBox::new("Scroll area").child(ScrollArea::new("scroll", 72.0).children(
                            (1..=12).map(|n| div().h(px(18.)).child(format!("Rule {n}: cell value greater than {}", n * 10))),
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(Button::new("alert", "Message box").on_click(cx.listener(|this, _, _, cx| {
                                this.modal.update_app(cx, |app, cx| {
                                    app.alert(
                                        Alert::new("OpenSheet", "This value doesn't match the data validation restrictions.")
                                            .stop()
                                            .button("Retry", |app, _, cx| app.set_status("Retry", cx))
                                            .button("Cancel", |app, _, cx| app.set_status("Cancel", cx)),
                                        cx,
                                    )
                                });
                            })))
                            .child(Button::new("pane", "Side pane").on_click(cx.listener(|this, _, _, cx| {
                                this.modal.update_app(cx, toggle_pane);
                            })))
                            .child(Button::new("popup", "Drop-down").on_click(cx.listener(|this, _, window, cx| {
                                this.modal.close(window, cx);
                                this.modal.update_app(cx, open_drop_down);
                            }))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        ColorGrid::new("fill", self.fill)
                            .none_label("No Color")
                            .more(&self.modal.app)
                            .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| {
                                this.fill = Some(*color);
                                cx.notify();
                            })),
                    )
                    .child(preview(196.0, 40.0, move |s: &mut Sketch| {
                        s.rect(0.0, 0.0, s.width, s.height, c(fill));
                        s.frame(0.0, 0.0, s.width, s.height, 1.0, c(theme::BORDER_STRONG));
                        s.text("Preview", 8.0, 12.0, &ui_font(), 12.0, c(theme::TEXT));
                    })),
            )
            .into_any_element()
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            1 => self.lists(cx),
            2 => self.more(cx),
            _ => self.basics(cx),
        };
        let modal = self.modal.clone();
        dialog(&self.modal, "Widgets")
            .width(580.0)
            .on_ok(move |_, window, cx| modal.close(window, cx))
            .child(
                TabStrip::new("tabs", names(&["Basics", "Lists", "More"]), self.tab).on_change(cx.listener(
                    |this, ix: &usize, _, cx| {
                        this.tab = *ix;
                        cx.notify();
                    },
                )),
            )
            .child(div().h(px(270.)).child(body))
            .footer(ok_cancel(&self.modal, {
                let modal = self.modal.clone();
                move |_, window, cx| modal.close(window, cx)
            }))
    }
}

/// The pane of the gallery: a tree, to try panes and their Tab order.
struct GalleryPane {
    tree: Vec<TreeNode>,
    checks: Vec<CheckItem>,
}

impl Render for GalleryPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(label("Fields:"))
            .child(
                div().flex().child(
                    Tree::new("pane-tree", &self.tree)
                        .plain()
                        .height(160.0)
                        .on_expand(cx.listener(|this, path: &Vec<usize>, _, cx| {
                            TreeNode::toggle_expanded(&mut this.tree, path);
                            cx.notify();
                        })),
                ),
            )
            .child(
                CheckList::new("pane-checks", self.checks.clone())
                    .height(140.0)
                    .on_toggle(cx.listener(|this, (ix, on): &(usize, bool), _, cx| {
                        this.checks[*ix].checked = *on;
                        cx.notify();
                    })),
            )
    }
}

fn toggle_pane(app: &mut ExcelApp, cx: &mut Cx) {
    if app.pane_open(PANE) {
        app.close_pane(PANE, cx);
        return;
    }
    let view = cx.new(|_| GalleryPane {
        tree: sample_tree(),
        checks: sample_checks(),
    });
    app.open_pane(PANE, "Widgets", view, cx);
}

/// A drop-down hanging from the active cell, as a filter button opens.
fn open_drop_down(app: &mut ExcelApp, cx: &mut Cx) {
    let (row, col) = app.active();
    let view = cx.new(|_| GalleryPane {
        tree: Vec::new(),
        checks: sample_checks(),
    });
    let list = cx.new(|_| DropDown { pane: view });
    app.open_popup_view(app.cell_anchor(Rect::cell(row, col)), list, cx);
}

struct DropDown {
    pane: Entity<GalleryPane>,
}

impl Render for DropDown {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(220.)).text_size(px(12.)).child(self.pane.clone())
    }
}

/// What the gallery adds to the grid while its pane is open.
pub struct GalleryMarks;

/// The zone on the corner of the active cell.
struct Corner;

impl Feature for GalleryMarks {
    fn name(&self) -> &'static str {
        "gallery"
    }

    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Over || !p.app.pane_open(PANE) {
            return;
        }
        let (row, col) = p.app.active();
        if !p.visible.contains(row, col) {
            return;
        }
        let (_, y0, x1, _) = p.cell_box(Rect::cell(row, col));
        let size = (7.0 * p.zoom()).max(5.0);
        p.polygon(&[(x1 - size - 1.0, y0), (x1 - 1.0, y0), (x1 - 1.0, y0 + size)], c(0xC00000));
        p.zone((x1 - size - 2.0, y0, x1, y0 + size + 1.0), Corner)
            .cursor(CursorStyle::PointingHand)
            .tooltip("A zone of the widget gallery: click it");
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        if zone.payload::<Corner>().is_none() {
            return ZoneResponse::Ignored;
        }
        let (row, col) = ev.cell;
        app.set_status(format!("Zone clicked on {}", Rect::cell(row, col).name()), cx);
        ZoneResponse::Handled
    }

    fn status_items(&self, app: &ExcelApp) -> Vec<StatusItem> {
        if app.pane_open(PANE) {
            vec![StatusItem::new("Widget gallery")]
        } else {
            Vec::new()
        }
    }
}
