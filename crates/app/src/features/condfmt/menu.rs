//! The drop-down of the Conditional Formatting button: Excel's menu with
//! its pictures, its submenus and the three galleries (data bars, colour
//! scales, icon sets). Resting on a thumbnail tries the rule on the sheet.
//!
//! The menu is a view shown with `ExcelApp::open_popup_view`. Its submenus
//! hang beside it, outside the box the application watches for "a press
//! elsewhere closes the drop-down": a press on a submenu therefore puts
//! the drop-down back before the click is delivered (`keep_open`), and the
//! feature holds the view so that it outlives that moment.

use super::{Cmd, Ui, add_rule, draw, end_preview, preview, words};
use crate::app::{ExcelApp, Popup, PopupKind};
use crate::features::{Command, Cx, Intent};
use crate::grid::Draw;
use crate::i18n::t;
use crate::theme::{self, c};
use crate::widgets::{icon, tooltip};
use gpui::{
    AnyElement, Bounds, Context, FontWeight, MouseButton, Pixels, Point, SharedString, WeakEntity, Window, canvas,
    div, fill, point, prelude::*, px, size,
};
use opensheet_engine::condfmt::{BarColor, CondFormat, DataBar, IconRule, IconSet, Rule, ScalePreset};
use opensheet_engine::style::{Color, StylePatch};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The entries that open a submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Sub {
    Highlight,
    TopBottom,
    DataBars,
    ColorScales,
    IconSets,
    Clear,
}

/// What a click on something of the menu does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Run(Cmd),
    Bar(BarColor, bool),
    Scale(ScalePreset),
    Icons(IconSet),
}

impl Act {
    /// The rule a gallery thumbnail stands for.
    fn rule(self) -> Option<Rule> {
        match self {
            Act::Run(_) => None,
            Act::Bar(color, gradient) => Some(Rule::DataBar(DataBar::preset(color, gradient))),
            Act::Scale(scale) => Some(Rule::ColorScale(scale.scale())),
            Act::Icons(set) => Some(Rule::IconSet(IconRule::preset(set))),
        }
    }
}

// Metrics of the panels.
const ROW_TALL: f32 = 40.0;
const ROW: f32 = 28.0;
const SEPARATOR: f32 = 9.0;
const PAD: f32 = 4.0;
const HEADING: f32 = 24.0;
const THUMB: f32 = 40.0;
const SET_ROW: f32 = 28.0;
const SET_COL: f32 = 112.0;

pub struct MenuView {
    app: WeakEntity<ExcelApp>,
    /// Where the drop-down was opened.
    pos: Point<Pixels>,
    width: f32,
    open: Option<Sub>,
    /// The thumbnail whose rule is being tried on the sheet.
    trying: Option<Act>,
    in_table: bool,
    in_pivot: bool,
    /// Where the panel and its submenu entries were last painted.
    panel: Rc<Cell<Bounds<Pixels>>>,
    rows: Rc<RefCell<HashMap<Sub, Bounds<Pixels>>>>,
}

/// Open the drop-down under the ribbon button, or close it when it is
/// open (or was closed by the press of this very click).
pub fn toggle(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let showing = showing(app);
    let state = app.ext_mut::<Ui>();
    let was_open = state.menu.take().is_some();
    let just_closed = state.menu_closed.take().is_some_and(|at| at.elapsed() < Duration::from_millis(500));
    if was_open || just_closed {
        end_preview(app, cx);
        if showing {
            app.close_popup(window, cx);
        }
        return;
    }
    let pos = match app.command_anchor.and_then(|anchor| app.anchors.borrow().get(anchor).copied()) {
        Some(bounds) => point(bounds.left(), bounds.bottom() + px(2.)),
        None => window.mouse_position(),
    };
    let (row, col) = app.active();
    let in_table = app.wb.table_at(app.sheet, row, col).is_some();
    let in_pivot = app.wb.pivot_at(app.sheet, row, col).is_some();
    let weak = cx.weak_entity();
    let view = cx.new(|_| MenuView {
        app: weak,
        pos,
        width: main_width(),
        open: None,
        trying: None,
        in_table,
        in_pivot,
        panel: Rc::default(),
        rows: Rc::default(),
    });
    app.ext_mut::<Ui>().menu = Some(view.clone());
    app.open_popup_view(pos, view, cx);
}

/// True while the application shows the drop-down.
fn showing(app: &ExcelApp) -> bool {
    let Some(view) = app.ext::<Ui>().and_then(|state| state.menu.as_ref()) else {
        return false;
    };
    matches!(&app.popup, Some(Popup { kind: PopupKind::View(shown), .. }) if shown.entity_id() == view.entity_id())
}

/// Called at every frame: once the application no longer shows the
/// drop-down, let go of it and of the rule a thumbnail was trying.
pub fn notice_closed(app: &mut ExcelApp, cx: &mut Cx) {
    if app.ext::<Ui>().is_some_and(|state| state.menu.is_some()) && !showing(app) {
        let state = app.ext_mut::<Ui>();
        state.menu = None;
        state.menu_closed = Some(Instant::now());
        end_preview(app, cx);
    }
}

/// Wide enough for the longest entry in the current language.
fn main_width() -> f32 {
    let longest = [
        t("Highlight Cells Rules", "Règles de mise en surbrillance des cellules"),
        t("Top/Bottom Rules", "Règles des valeurs de plage haute/basse"),
    ]
    .iter()
    .map(|label| label.chars().count())
    .max()
    .unwrap_or(0);
    (longest as f32 * 6.6 + 84.0).clamp(250.0, 400.0)
}

/// Width of a submenu of plain entries.
fn text_width(commands: &[Cmd]) -> f32 {
    let longest = commands.iter().map(|cmd| cmd.label().chars().count()).max().unwrap_or(0);
    (longest as f32 * 6.6 + 70.0).clamp(190.0, 400.0)
}

const HIGHLIGHT: [Cmd; 7] = [
    Cmd::GreaterThan,
    Cmd::LessThan,
    Cmd::Between,
    Cmd::EqualTo,
    Cmd::TextContains,
    Cmd::DateOccurring,
    Cmd::DuplicateValues,
];
const TOP_BOTTOM: [Cmd; 6] = [
    Cmd::Top10Items,
    Cmd::Top10Percent,
    Cmd::Bottom10Items,
    Cmd::Bottom10Percent,
    Cmd::AboveAverage,
    Cmd::BelowAverage,
];
const CLEAR: [Cmd; 4] = [Cmd::ClearSelected, Cmd::ClearSheet, Cmd::ClearTable, Cmd::ClearPivot];

impl Sub {
    /// Width and height of the submenu.
    fn size(self) -> (f32, f32) {
        let more = SEPARATOR + ROW;
        match self {
            Sub::Highlight => (text_width(&HIGHLIGHT), 2.0 * PAD + 7.0 * ROW_TALL + more),
            Sub::TopBottom => (text_width(&TOP_BOTTOM), 2.0 * PAD + 6.0 * ROW_TALL + more),
            Sub::DataBars => (3.0 * (THUMB + 4.0) + 2.0 * PAD + 8.0, 2.0 * PAD + 2.0 * HEADING + 4.0 * (THUMB + 4.0) + more),
            Sub::ColorScales => (4.0 * (THUMB + 4.0) + 2.0 * PAD + 8.0, 2.0 * PAD + 3.0 * (THUMB + 4.0) + more),
            Sub::IconSets => {
                let rows: usize = words::set_groups().iter().map(|(_, count)| count.div_ceil(2)).sum();
                (2.0 * SET_COL + 2.0 * PAD + 8.0, 2.0 * PAD + 4.0 * HEADING + rows as f32 * SET_ROW + more)
            }
            Sub::Clear => (text_width(&CLEAR), 2.0 * PAD + 4.0 * ROW),
        }
    }
}

/// A small sheet with a highlighted cell and a sign: the picture of the
/// entries that have none of their own.
fn sign_tile(sign: &'static str) -> AnyElement {
    div()
        .size(px(28.))
        .flex_none()
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(0xB5B5B5))
        .rounded(px(2.))
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .h(px(16.))
                .bg(c(0xFFC7CE))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(if sign.chars().count() > 2 { 9.5 } else { 12.0 }))
                .font_weight(FontWeight::BOLD)
                .text_color(c(0x9C0006))
                .child(sign),
        )
        .into_any_element()
}

fn command_sign(cmd: Cmd) -> &'static str {
    match cmd {
        Cmd::GreaterThan => ">",
        Cmd::LessThan => "<",
        Cmd::Between => "↔",
        Cmd::EqualTo => "=",
        Cmd::TextContains => "ab",
        Cmd::DateOccurring => "31",
        Cmd::DuplicateValues => "≡",
        Cmd::Top10Items => "↑10",
        Cmd::Top10Percent => "↑%",
        Cmd::Bottom10Items => "↓10",
        Cmd::Bottom10Percent => "↓%",
        Cmd::AboveAverage => "x̄↑",
        Cmd::BelowAverage => "x̄↓",
        _ => "",
    }
}

/// A frame the size of a thumbnail with `draw` painting inside it.
fn framed(draw: impl Fn(Bounds<Pixels>, &mut Vec<Draw>) + 'static) -> AnyElement {
    div()
        .size(px(28.))
        .flex_none()
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(0xB5B5B5))
        .rounded(px(2.))
        .child(draw::picture(26.0, 26.0, draw))
        .into_any_element()
}

/// Three bars of different lengths: a data bar rule in small.
fn bars_thumbnail(color: Color, gradient: bool) -> AnyElement {
    framed(move |bounds, out| {
        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        for (i, share) in [0.9f32, 0.55, 0.3].into_iter().enumerate() {
            let top = y + 2.0 + i as f32 * 8.0;
            let bar = opensheet_engine::condfmt::Bar {
                start: 0.0,
                end: share,
                fill: color,
                gradient,
                border: gradient.then_some(color),
                negative: false,
                axis: None,
                axis_color: Color::BLACK,
            };
            draw::bar(&bar, (x, top - 1.0, x + 27.0, top + 8.0), 1.0, out);
        }
    })
}

/// The colours of a scale as the rows of a small sheet, the colour of the
/// highest values on top.
fn scale_thumbnail(colors: Vec<Color>) -> AnyElement {
    framed(move |bounds, out| {
        let rows: Vec<Color> = match colors[..] {
            [low, high] => vec![high, mix(low, high), low],
            _ => colors.iter().rev().copied().collect(),
        };
        let (x, y) = (bounds.origin.x, bounds.origin.y);
        for (i, color) in rows.iter().enumerate() {
            let top = y + px(1.0 + i as f32 * 8.0);
            out.push(Draw::Quad(fill(Bounds::new(point(x + px(1.), top), size(px(24.), px(7.))), c(color.value()))));
        }
    })
}

/// Half-way between two colours.
fn mix(a: Color, b: Color) -> Color {
    let half = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    Color::rgb(half(a.r(), b.r()), half(a.g(), b.g()), half(a.b(), b.b()))
}

impl MenuView {
    fn app_mut(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut ExcelApp, &mut Cx)) {
        self.app.update(cx, f).ok();
    }

    /// A press on a submenu closed the drop-down (see the module comment):
    /// show it again as it was.
    fn keep_open(&mut self, cx: &mut Context<Self>) {
        let (view, pos) = (cx.entity(), self.pos);
        self.app_mut(cx, |app, cx| {
            if app.popup.is_none() && !app.modal_open() {
                app.popup = Some(Popup::new(PopupKind::View(view.into()), pos, 0.0).max_height(600.0));
                cx.notify();
            }
        });
    }

    fn pick(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        self.trying = None;
        self.app_mut(cx, |app, cx| {
            app.ext_mut::<Ui>().menu = None;
            end_preview(app, cx);
            app.close_popup(window, cx);
            match act {
                Act::Run(cmd) => cmd.run(app, window, cx),
                gallery => {
                    if app.is_editing() && !app.commit_edit(None, window, cx) {
                        return;
                    }
                    if let (Some(rule), true) = (gallery.rule(), app.may_all(Intent::FormatCells, cx)) {
                        add_rule(app, rule, StylePatch::default(), cx);
                    }
                }
            }
        });
    }

    /// The pointer came on a thumbnail or left it: try its rule on the
    /// sheet, or show the sheet as it is again.
    fn hover(&mut self, act: Act, over: bool, cx: &mut Context<Self>) {
        if over {
            self.trying = Some(act);
            self.app_mut(cx, |app, cx| {
                let rule = act.rule().map(|rule| CondFormat::new(app.sel().ranges.clone(), rule));
                preview(app, rule, cx);
            });
        } else if self.trying == Some(act) {
            self.trying = None;
            self.app_mut(cx, end_preview);
        }
    }

    fn set_open(&mut self, sub: Option<Sub>, cx: &mut Context<Self>) {
        if self.open != sub {
            self.open = sub;
            cx.notify();
        }
    }

    /// An entry of the main panel or of a submenu of plain entries.
    fn entry(
        &self,
        id: usize,
        picture: Option<AnyElement>,
        label: &'static str,
        sub: Option<Sub>,
        act: Option<Act>,
        enabled: bool,
        height: f32,
        main: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = main && sub.is_some() && self.open == sub;
        let rows = self.rows.clone();
        div()
            .id(("cf-entry", id))
            .relative()
            .h(px(height))
            .mx(px(PAD))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .text_size(px(12.5))
            .text_color(c(if enabled { theme::TEXT } else { theme::TEXT_DISABLED }))
            .when(open, |d| d.bg(c(theme::HOVER)))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .child(div().w(px(28.)).flex_none().flex().justify_center().children(picture))
            .child(div().flex_1().truncate().child(label))
            .when_some(sub, |d, sub| {
                d.child(icon("chevron-right", 10.0, theme::TEXT_MUTED)).child(
                    canvas(
                        move |bounds, _, _| {
                            rows.borrow_mut().insert(sub, bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
            // Resting on an entry of the main panel opens its submenu and
            // closes the one that was open.
            .when(main, |d| d.on_mouse_move(cx.listener(move |this, _, _, cx| this.set_open(sub, cx))))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !enabled {
                    return;
                }
                match (sub, act) {
                    (Some(sub), _) => this.set_open(Some(sub), cx),
                    (None, Some(act)) => this.pick(act, window, cx),
                    (None, None) => {}
                }
            }))
            .into_any_element()
    }

    fn separator() -> AnyElement {
        div()
            .h(px(1.))
            .my(px((SEPARATOR - 1.0) / 2.0))
            .mx(px(8.))
            .bg(c(theme::BORDER))
            .into_any_element()
    }

    fn heading(text: &'static str) -> AnyElement {
        div()
            .h(px(HEADING))
            .px(px(8.))
            .flex()
            .items_center()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(theme::TEXT_MUTED))
            .child(text)
            .into_any_element()
    }

    /// A thumbnail of a gallery: a click applies its rule, resting on it
    /// tries the rule on the sheet.
    fn thumbnail(
        &self,
        id: usize,
        width: f32,
        height: f32,
        picture: AnyElement,
        name: &'static str,
        act: Act,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("cf-thumb", id))
            .w(px(width))
            .h(px(height))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .border_1()
            .border_color(gpui::transparent_black())
            .hover(|s| s.bg(c(theme::HOVER)).border_color(c(theme::BORDER_STRONG)))
            .child(picture)
            .tooltip(tooltip(SharedString::from(name)))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| this.hover(act, *over, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.pick(act, window, cx)))
            .into_any_element()
    }

    fn more(&self, cmd: Cmd, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            Self::separator(),
            self.entry(900, None, cmd.label(), None, Some(Act::Run(cmd)), true, ROW, false, cx),
        ]
    }

    fn command_rows(&self, commands: &[Cmd], height: f32, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let app = self.app.upgrade();
        commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let enabled = match cmd {
                    Cmd::ClearTable => self.in_table,
                    Cmd::ClearPivot => self.in_pivot,
                    _ => app.is_some(),
                };
                let sign = command_sign(*cmd);
                let picture = (!sign.is_empty()).then(|| sign_tile(sign));
                self.entry(100 + i, picture, cmd.label(), None, Some(Act::Run(*cmd)), enabled, height, false, cx)
            })
            .collect()
    }

    fn submenu(&self, sub: Sub, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let grid = |cells: Vec<AnyElement>, columns: usize| -> Vec<AnyElement> {
            let mut rows = Vec::new();
            let mut cells = cells.into_iter().peekable();
            while cells.peek().is_some() {
                let row: Vec<AnyElement> = cells.by_ref().take(columns).collect();
                rows.push(div().px(px(PAD + 4.0)).flex().children(row).into_any_element());
            }
            rows
        };
        match sub {
            Sub::Highlight => {
                let mut out = self.command_rows(&HIGHLIGHT, ROW_TALL, cx);
                out.extend(self.more(Cmd::MoreHighlight, cx));
                out
            }
            Sub::TopBottom => {
                let mut out = self.command_rows(&TOP_BOTTOM, ROW_TALL, cx);
                out.extend(self.more(Cmd::MoreTopBottom, cx));
                out
            }
            Sub::Clear => self.command_rows(&CLEAR, ROW, cx),
            Sub::DataBars => {
                let mut out = Vec::new();
                for (g, (gradient, title)) in [
                    (true, t("Gradient Fill", "Dégradé")),
                    (false, t("Solid Fill", "Remplissage uni")),
                ]
                .into_iter()
                .enumerate()
                {
                    out.push(Self::heading(title));
                    let cells = BarColor::ALL
                        .iter()
                        .enumerate()
                        .map(|(i, color)| {
                            self.thumbnail(
                                g * 10 + i,
                                THUMB + 4.0,
                                THUMB + 4.0,
                                bars_thumbnail(color.color(), gradient),
                                words::bar_color_name(*color),
                                Act::Bar(*color, gradient),
                                cx,
                            )
                        })
                        .collect();
                    out.extend(grid(cells, 3));
                }
                out.extend(self.more(Cmd::DataBars, cx));
                out
            }
            Sub::ColorScales => {
                let cells = ScalePreset::ALL
                    .iter()
                    .enumerate()
                    .map(|(i, scale)| {
                        self.thumbnail(
                            100 + i,
                            THUMB + 4.0,
                            THUMB + 4.0,
                            scale_thumbnail(scale.colors()),
                            words::scale_name(*scale),
                            Act::Scale(*scale),
                            cx,
                        )
                    })
                    .collect();
                let mut out = grid(cells, 4);
                out.extend(self.more(Cmd::ColorScales, cx));
                out
            }
            Sub::IconSets => {
                let mut out = Vec::new();
                let mut sets = IconSet::ALL.iter().enumerate();
                for (title, count) in words::set_groups() {
                    out.push(Self::heading(title));
                    let cells = sets
                        .by_ref()
                        .take(count)
                        .map(|(i, set)| {
                            let picture = div()
                                .w(px(SET_COL - 12.0))
                                .child(draw::set_picture(*set, 16.0, 3.0))
                                .into_any_element();
                            self.thumbnail(200 + i, SET_COL, SET_ROW, picture, words::set_name(*set), Act::Icons(*set), cx)
                        })
                        .collect();
                    out.extend(grid(cells, 2));
                }
                out.extend(self.more(Cmd::IconSets, cx));
                out
            }
        }
    }
}

impl Render for MenuView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bars = framed(|bounds, out| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            for (i, share) in [0.85f32, 0.5, 0.65].into_iter().enumerate() {
                let top = y + 2.0 + i as f32 * 8.0;
                let color = Color(0x638EC6);
                let bar = opensheet_engine::condfmt::Bar {
                    start: 0.0,
                    end: share,
                    fill: color,
                    gradient: true,
                    border: Some(color),
                    negative: false,
                    axis: None,
                    axis_color: Color::BLACK,
                };
                draw::bar(&bar, (x, top - 1.0, x + 27.0, top + 8.0), 1.0, out);
            }
        });
        let scales = scale_thumbnail(ScalePreset::GreenYellowRed.colors());
        let icons = framed(|bounds, out| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            for (i, which) in IconSet::ThreeSigns.icons().iter().rev().enumerate() {
                draw::icon(*which, x + 9.0, y + 1.0 + i as f32 * 8.5, 7.5, out);
            }
        });
        let small = |name: &'static str| Some(icon(name, 16.0, theme::TEXT_MUTED).into_any_element());
        let main: Vec<AnyElement> = vec![
            self.entry(
                0,
                Some(sign_tile("≤")),
                t("Highlight Cells Rules", "Règles de mise en surbrillance des cellules"),
                Some(Sub::Highlight),
                None,
                true,
                ROW_TALL,
                true,
                cx,
            ),
            self.entry(
                1,
                Some(sign_tile("10")),
                t("Top/Bottom Rules", "Règles des valeurs de plage haute/basse"),
                Some(Sub::TopBottom),
                None,
                true,
                ROW_TALL,
                true,
                cx,
            ),
            Self::separator(),
            self.entry(2, Some(bars), t("Data Bars", "Barres de données"), Some(Sub::DataBars), None, true, ROW_TALL, true, cx),
            self.entry(
                3,
                Some(scales),
                t("Color Scales", "Nuances de couleurs"),
                Some(Sub::ColorScales),
                None,
                true,
                ROW_TALL,
                true,
                cx,
            ),
            self.entry(4, Some(icons), t("Icon Sets", "Jeux d'icônes"), Some(Sub::IconSets), None, true, ROW_TALL, true, cx),
            Self::separator(),
            self.entry(5, small("condfmt-new"), Cmd::NewRule.label(), None, Some(Act::Run(Cmd::NewRule)), true, ROW, true, cx),
            self.entry(
                6,
                small("condfmt-clear"),
                t("Clear Rules", "Effacer les règles"),
                Some(Sub::Clear),
                None,
                true,
                ROW,
                true,
                cx,
            ),
            self.entry(7, small("condfmt-manage"), Cmd::Manage.label(), None, Some(Act::Run(Cmd::Manage)), true, ROW, true, cx),
        ];

        // The open submenu hangs beside its entry: to the right while
        // there is room, and never below the bottom of the window.
        let submenu = self.open.and_then(|sub| {
            let panel = self.panel.get();
            let row = self.rows.borrow().get(&sub).copied()?;
            let (width, height) = sub.size();
            let viewport = window.viewport_size();
            let fits_right = panel.right() + px(width + 8.0) <= viewport.width;
            let left = if fits_right || panel.left() < px(width + 8.0) { px(self.width + 2.0) } else { px(-width - 4.0) };
            let lowest = viewport.height - px(height + 8.0) - panel.top();
            let top = (row.top() - panel.top() - px(PAD + 1.0)).min(lowest);
            Some(
                div()
                    .id("cf-submenu")
                    .absolute()
                    .left(left)
                    .top(top)
                    .w(px(width))
                    .occlude()
                    .py(px(PAD))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .rounded(px(6.))
                    .shadow_lg()
                    .on_any_mouse_down(cx.listener(|this, _, _, cx| this.keep_open(cx)))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .children(self.submenu(sub, cx)),
            )
        });

        let bounds = self.panel.clone();
        div()
            .id("cf-menu")
            .relative()
            .w(px(self.width))
            .py(px(PAD))
            .text_color(c(theme::TEXT))
            .children(main)
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .children(submenu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gallery_thumbnail_makes_its_rule() {
        assert!(matches!(Act::Bar(BarColor::Red, true).rule(), Some(Rule::DataBar(bar)) if bar.gradient));
        assert!(matches!(Act::Bar(BarColor::Red, false).rule(), Some(Rule::DataBar(bar)) if bar.border.is_none()));
        assert!(matches!(Act::Scale(ScalePreset::WhiteRed).rule(), Some(Rule::ColorScale(scale)) if scale.stops.len() == 2));
        assert!(matches!(Act::Icons(IconSet::FiveBoxes).rule(), Some(Rule::IconSet(icons)) if icons.thresholds.len() == 5));
        assert_eq!(Act::Run(Cmd::NewRule).rule(), None);
    }

    #[test]
    fn the_galleries_hold_what_excel_shows() {
        assert_eq!(BarColor::ALL.len() * 2, 12);
        assert_eq!(ScalePreset::ALL.len(), 12);
        assert_eq!(IconSet::ALL.len(), 20);
        // Every submenu has a size, the icon sets being the tallest.
        let heights: Vec<f32> = [Sub::Highlight, Sub::TopBottom, Sub::DataBars, Sub::ColorScales, Sub::IconSets, Sub::Clear]
            .iter()
            .map(|sub| sub.size().1)
            .collect();
        assert!(heights.iter().all(|h| *h > 100.0));
        assert_eq!(heights.iter().copied().fold(0.0, f32::max), heights[4]);
    }

    #[test]
    fn a_two_colour_scale_shows_its_middle() {
        assert_eq!(mix(Color(0x000000), Color(0xFFFFFF)), Color(0x7F7F7F));
    }
}
