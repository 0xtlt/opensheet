//! Rendering of the window: title bar, formula bar, grid host, sheet tabs,
//! status bar and pop-ups. The ribbon lives in `ribbon.rs`, dialogs and
//! side panes in `shell.rs`.

use crate::app::*;
use crate::features::{self, ContextMenu, Intent, MenuSection, MenuTarget, ViewMode};
use crate::grid::{self, SceneInput, pt_to_px};
use crate::i18n::t;
use crate::icons::icon_path;
use crate::theme::{self, c, ca};
use gpui::{
    AnyElement, AnyView, App, Context, DispatchPhase, ElementInputHandler, FontWeight, HitboxBehavior, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, SharedString, Stateful, TextRun, Window, anchored,
    canvas, deferred, div, prelude::*, px, svg,
};
use opensheet_engine::Rect;
use opensheet_engine::refs::MAX_COLS;
use opensheet_engine::style::Color;

/// A small text bubble shown when hovering a control.
pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(4.))
            .shadow_md()
            .px_2()
            .py_1()
            .text_size(px(12.))
            .text_color(c(theme::TEXT))
            .child(self.0.clone())
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

pub fn icon(name: &str, size: f32, color: u32) -> gpui::Svg {
    svg()
        .path(icon_path(name))
        .size(px(size))
        .flex_none()
        .text_color(c(color))
}

impl ExcelApp {
    /// Records the bounds of the element it is placed in (which must be
    /// `relative`), so a drop-down can be anchored to it later.
    pub fn anchor(&self, id: &'static str) -> impl IntoElement + use<> {
        let anchors = self.anchors.clone();
        canvas(
            move |bounds, _, _| {
                anchors.borrow_mut().insert(id, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn titlebar_button(
        &self,
        id: &'static str,
        icon_name: &'static str,
        tip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::TITLEBAR_HOVER))))
            .child(icon(icon_name, 16.0, if enabled { 0xFFFFFF } else { 0x7FBF9B }))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    f(this, window, cx)
                }
            }))
    }

    fn render_titlebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = format!(
            "{}{}",
            self.title,
            if self.wb.modified { "" } else { "" }
        );
        div()
            .id("titlebar")
            .h(px(theme::TITLEBAR_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .bg(c(theme::TITLEBAR))
            .text_color(c(0xFFFFFF))
            .text_size(px(13.))
            .on_click(cx.listener(|_, ev: &gpui::ClickEvent, window, _| {
                if ev.click_count() == 2 {
                    window.titlebar_double_click();
                }
            }))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    // Room for the traffic lights.
                    .pl(px(84.))
                    .child(self.titlebar_button(
                        "tb-save",
                        "save",
                        t("Save (⌘S)", "Enregistrer (⌘S)"),
                        true,
                        cx,
                        |this, window, cx| this.save(false, window, cx),
                    ))
                    .child(self.titlebar_button(
                        "tb-undo",
                        "undo",
                        t("Undo (⌘Z)", "Annuler (⌘Z)"),
                        self.wb.can_undo(),
                        cx,
                        |this, _, cx| this.undo(cx),
                    ))
                    .child(self.titlebar_button(
                        "tb-redo",
                        "redo",
                        t("Redo (⌘Y)", "Rétablir (⌘Y)"),
                        self.wb.can_redo(),
                        cx,
                        |this, _, cx| this.redo(cx),
                    ))
                    .child(div().pl_3().font_weight(FontWeight::MEDIUM).child(title))
                    .child(
                        div()
                            .pl_1()
                            .text_color(ca(0xFFFFFF, 0.75))
                            .child(if self.wb.modified {
                                t("— Edited", "— Modifié")
                            } else if self.path.is_some() {
                                t("— Saved", "— Enregistré")
                            } else {
                                ""
                            }),
                    ),
            )
            .child(
                div()
                    .id("tb-search")
                    .w(px(360.))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .rounded(px(5.))
                    .bg(ca(0xFFFFFF, 0.88))
                    .text_color(c(theme::TEXT_MUTED))
                    .text_size(px(12.))
                    .child(icon("search", 14.0, theme::TEXT_MUTED))
                    .child(t("Search (⌘F)", "Rechercher (⌘F)"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_find(window, cx))),
            )
            .child(
                div().flex_1().flex().items_center().justify_end().pr_3().child(
                    div()
                        .size(px(26.))
                        .rounded_full()
                        .bg(ca(0xFFFFFF, 0.22))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("OS"),
                ),
            )
    }

    fn render_formula_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.editing.is_some();
        let small = |id: &'static str, icon_name: &'static str, color: u32| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(icon(icon_name, 14.0, color))
        };
        div()
            .h(px(theme::FORMULA_BAR_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .bg(c(theme::CHROME_BG))
            .px_2()
            .gap_1()
            .child(
                div()
                    .w(px(120.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .rounded(px(3.))
                    .px_2()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .child(self.name_box.clone())
                    .child(icon("chevron-down", 12.0, theme::TEXT_MUTED)),
            )
            .child(
                small("fb-cancel", "close", if editing { 0xC42B1C } else { theme::TEXT_DISABLED })
                    .tooltip(tooltip(t("Cancel (Esc)", "Annuler (Échap)")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.is_editing() {
                            this.cancel_edit(window, cx)
                        }
                    })),
            )
            .child(
                small("fb-enter", "check", if editing { theme::ACCENT } else { theme::TEXT_DISABLED })
                    .tooltip(tooltip(t("Enter", "Entrer")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.is_editing() {
                            this.commit_edit(None, window, cx);
                        }
                    })),
            )
            .child(
                small("fb-fx", "function", theme::TEXT_MUTED)
                    .tooltip(tooltip(t("Insert Function", "Insérer une fonction")))
                    .on_click(cx.listener(|this, _, window, cx| this.open_function_menu("fb-fx-anchor", None, window, cx)))
                    .relative()
                    .child(self.anchor("fb-fx-anchor")),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .rounded(px(3.))
                    .px_2()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .overflow_hidden()
                    .child(self.formula_input.clone()),
            )
    }

    fn render_cell_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.editing.as_ref()?;
        if edit.sheet != self.sheet {
            return None;
        }
        let layout = self.layout.as_ref().map(|l| self.make_layout(l.bounds))?;
        let sheet = &self.wb.sheets[self.sheet];
        let rect = sheet.merge_at(edit.row, edit.col).unwrap_or(Rect::cell(edit.row, edit.col));
        let (x0, y0, x1, y1) = layout.rect_px(rect);
        // As drawn: with the table style and conditional formats applied.
        let style = self.wb.display(self.sheet, edit.row, edit.col).style.into_owned();
        let font_px = (pt_to_px(style.font.size_pt()) * self.zoom).max(1.0);
        let line_h = (font_px * 1.22).round();
        let font = grid::cell_font(&style, &self.fonts);
        let text = self.cell_editor.read(cx).text().replace('\n', " ");
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: c(0),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text_w = f32::from(
            window
                .text_system()
                .shape_line(text.into(), px(font_px), &[run], None)
                .width,
        );
        let pad = (3.0 * self.zoom).round().max(1.0);
        // The editor is as wide as its cell, and wider when the text needs it.
        let cell_w = (x1 - x0 - 1.0).max(8.0);
        let available = (layout.width() - 2.0 - x0).max(cell_w);
        let needed = text_w + 2.0 * pad + 4.0;
        let mut width = needed.max(cell_w).min(available);
        // Does the overflow run over cells that show something?
        let mut covers_content = false;
        if width > cell_w {
            let mut col = rect.c1 + 1;
            let mut edge = x1;
            while edge < x0 + width && col < MAX_COLS {
                if !sheet.is_blank(edit.row, col) || sheet.merge_at(edit.row, col).is_some() {
                    covers_content = true;
                    break;
                }
                edge += layout.col_w(col);
                col += 1;
            }
        }
        if covers_content {
            // Like Excel, hide whole neighbouring cells rather than cut
            // their content in half: grow to the next column boundary.
            let mut right = x1;
            let mut col = rect.c1;
            while right - x0 < needed && col + 1 < MAX_COLS && right < layout.width() {
                col += 1;
                right += layout.col_w(col);
            }
            width = (right - x0 - 1.0).max(cell_w).min(available);
        }
        // Over blank neighbours the editor stays see-through, so a cell the
        // formula refers to keeps its coloured highlight under the text.
        let backdrop_w = if covers_content { width } else { cell_w };
        let h = (y1 - y0 - 1.0).max(line_h + 1.0);
        let bg = style.fill.map(theme::engine_color).unwrap_or(c(0xFFFFFF));
        let color = style.font.color.map(theme::engine_color).unwrap_or(c(0x000000));
        Some(
            div()
                .absolute()
                .left(px(x0))
                .top(px(y0))
                .w(px(width))
                .h(px(h))
                .occlude()
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(px(backdrop_w))
                        .h_full()
                        .bg(bg),
                )
                .flex()
                .items_end()
                .px(px(pad))
                .pb(px((1.0 * self.zoom).round().max(1.0)))
                .font(font)
                .text_size(px(font_px))
                .line_height(px(line_h))
                .text_color(color)
                // A click in the box but off the line of text still places
                // the caret.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.cell_editor
                            .update(cx, |input, cx| input.caret_to_point(ev.position, cx));
                    }),
                )
                .child(self.cell_editor.clone())
                .into_any_element(),
        )
    }

    fn render_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let grid_focus = self.grid_focus.clone();
        let editor = self.render_cell_editor(window, cx);
        // What features place over the cells (a note being typed, ...).
        let overlays = match self.current_layout() {
            Some(layout) => features::overlay_elements(self, &layout, window, cx),
            None => Vec::new(),
        };
        let zone_tip = self.zone_tip.clone().map(|(position, text)| {
            deferred(
                anchored()
                    .position(position + gpui::point(px(12.), px(18.)))
                    .snap_to_window_with_margin(px(6.))
                    .child(
                        div()
                            .max_w(px(320.))
                            .bg(c(0xFFFFFF))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .rounded(px(4.))
                            .shadow_md()
                            .px_2()
                            .py_1()
                            .text_size(px(12.))
                            .text_color(c(theme::TEXT))
                            .child(text),
                    ),
            )
            .with_priority(1)
        });
        let prepaint_entity = entity.clone();
        div()
            .id("grid")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_hidden()
            .track_focus(&self.grid_focus)
            .key_context("Grid")
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_extend_up))
            .on_action(cx.listener(Self::on_extend_down))
            .on_action(cx.listener(Self::on_extend_left))
            .on_action(cx.listener(Self::on_extend_right))
            .on_action(cx.listener(Self::on_jump_up))
            .on_action(cx.listener(Self::on_jump_down))
            .on_action(cx.listener(Self::on_jump_left))
            .on_action(cx.listener(Self::on_jump_right))
            .on_action(cx.listener(Self::on_extend_jump_up))
            .on_action(cx.listener(Self::on_extend_jump_down))
            .on_action(cx.listener(Self::on_extend_jump_left))
            .on_action(cx.listener(Self::on_extend_jump_right))
            .on_action(cx.listener(Self::on_next_cell))
            .on_action(cx.listener(Self::on_prev_cell))
            .on_action(cx.listener(Self::on_enter_down))
            .on_action(cx.listener(Self::on_enter_up))
            .on_action(cx.listener(Self::on_edit_cell))
            .on_action(cx.listener(Self::on_clear_contents))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_paste_values))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_fill_down))
            .on_action(cx.listener(Self::on_fill_right))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_go_row_start))
            .on_action(cx.listener(Self::on_go_start))
            .on_action(cx.listener(Self::on_go_end))
            .on_action(cx.listener(Self::on_select_column))
            .on_action(cx.listener(Self::on_select_row))
            .on_action(cx.listener(Self::on_insert_date))
            .on_action(cx.listener(Self::on_auto_sum))
            .on_action(cx.listener(Self::on_insert_rows))
            .on_action(cx.listener(Self::on_delete_rows))
            .on_action(cx.listener(Self::on_open_drop_down))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                        let ts = window.text_system().clone();
                        let scale = window.scale_factor();
                        let app = prepaint_entity.read(cx);
                        let layout = app.make_layout(bounds);
                        let (content_w, content_h) = app.content_size(&layout);
                        let marquee = app
                            .clip
                            .as_ref()
                            .filter(|s| s.clip.sheet == app.sheet)
                            .map(|s| s.clip.rect);
                        // The cells being picked for a dialog are outlined
                        // like copied ones.
                        let marquee = app.pick.as_ref().map(|pick| pick.rect).or(marquee);
                        let input = SceneInput {
                            app,
                            wb: &app.wb,
                            sheet: app.sheet,
                            selection: app.sel(),
                            fonts: &app.fonts,
                            marquee,
                            fill_preview: app.fill_preview(),
                            ref_highlights: if app.is_editing() { &app.ref_highlights } else { &[] },
                            show_fill_handle: !app.is_editing() && app.object_sel.is_none(),
                            content_w,
                            content_h,
                            hot_scrollbar: app.hot_scrollbar(),
                            scale,
                        };
                        let scene = grid::build_scene(&input, &layout, &ts);
                        let cursor = app.cursor_style();
                        (hitbox, layout, scene, cursor)
                    },
                    move |bounds, (hitbox, layout, scene, cursor), window, cx| {
                        window.handle_input(&grid_focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                        grid::paint_layers(scene.layers, window, cx);
                        window.set_cursor_style(cursor, &hitbox);
                        entity.update(cx, |app, _| {
                            app.layout = Some(layout);
                            app.zones = scene.zones;
                        });

                        window.on_mouse_event({
                            let entity = entity.clone();
                            let hitbox = hitbox.clone();
                            move |ev: &MouseDownEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                                    entity.update(cx, |app, cx| app.grid_mouse_down(ev, window, cx));
                                }
                            }
                        });
                        window.on_mouse_event({
                            let entity = entity.clone();
                            let hitbox = hitbox.clone();
                            move |ev: &MouseMoveEvent, phase, window, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let dragging = entity.read(cx).is_dragging();
                                if dragging || hitbox.is_hovered(window) {
                                    entity.update(cx, |app, cx| app.grid_mouse_move(ev, window, cx));
                                } else {
                                    entity.update(cx, |app, cx| app.clear_hover(cx));
                                }
                            }
                        });
                        window.on_mouse_event({
                            let entity = entity.clone();
                            move |ev: &MouseUpEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble && ev.button == MouseButton::Left {
                                    let dragging = entity.read(cx).is_dragging();
                                    if dragging {
                                        entity.update(cx, |app, cx| app.grid_mouse_up(ev, window, cx));
                                    }
                                }
                            }
                        });
                        window.on_mouse_event({
                            let entity = entity.clone();
                            let hitbox = hitbox.clone();
                            move |ev: &ScrollWheelEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                                    entity.update(cx, |app, cx| app.grid_scroll(ev, window, cx));
                                }
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(editor)
            .children(overlays)
            .children(zone_tip)
    }

    fn render_sheet_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<AnyElement> = Vec::new();
        for (i, sheet) in self.wb.sheets.iter().enumerate() {
            let active = i == self.sheet;
            let renaming = self.renaming_sheet == Some(i);
            let underline = sheet.tab_color.map(theme::engine_color).unwrap_or(c(theme::ACCENT));
            let tab = div()
                .id(("sheet-tab", i))
                .h_full()
                .flex()
                .items_center()
                .px(px(14.))
                .min_w(px(56.))
                .text_size(px(12.))
                .border_b_2()
                .when(active, |d| {
                    d.bg(c(0xFFFFFF))
                        .border_color(underline)
                        .text_color(c(theme::ACCENT_DARK))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |d| {
                    d.border_color(if sheet.tab_color.is_some() {
                        underline
                    } else {
                        gpui::transparent_black()
                    })
                    .text_color(c(theme::TEXT_MUTED))
                    .hover(|s| s.bg(c(theme::HOVER)))
                })
                .when(renaming, |d| {
                    d.child(
                        div()
                            .min_w(px(70.))
                            .line_height(px(18.))
                            .bg(c(0xFFFFFF))
                            .child(self.rename_input.clone()),
                    )
                })
                .when(!renaming, |d| d.child(sheet.name.clone()))
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        this.begin_rename_sheet(i, window, cx);
                    } else {
                        this.switch_sheet(i, window, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        this.switch_sheet(i, window, cx);
                        this.open_sheet_menu(i, ev.position, cx);
                    }),
                );
            tabs.push(tab.into_any_element());
        }
        let nav = |id: &'static str, icon_name: &'static str| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(icon(icon_name, 12.0, theme::TEXT_MUTED))
        };
        div()
            .h(px(theme::SHEET_TABS_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .bg(c(theme::CHROME_BG))
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_2()
                    .gap_1()
                    .child(nav("tabs-prev", "chevron-left").on_click(cx.listener(|this, _, window, cx| {
                        let i = this.sheet.saturating_sub(1);
                        this.switch_sheet(i, window, cx)
                    })))
                    .child(nav("tabs-next", "chevron-right").on_click(cx.listener(|this, _, window, cx| {
                        let i = (this.sheet + 1).min(this.wb.sheets.len() - 1);
                        this.switch_sheet(i, window, cx)
                    }))),
            )
            .child(div().h_full().flex().items_center().children(tabs))
            .child(
                div().px_2().child(
                    nav("tabs-add", "plus")
                        .tooltip(tooltip(t("New sheet", "Nouvelle feuille")))
                        .on_click(cx.listener(|this, _, _, cx| this.add_sheet(cx))),
                ),
            )
            .child(div().flex_1())
    }

    fn open_sheet_menu(&mut self, index: usize, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let only = self.wb.sheets.len() <= 1;
        let last = index + 1 >= self.wb.sheets.len();
        let mut menu = ContextMenu::default();
        let mut add = |section: MenuSection, entry: MenuEntry| menu.add(section, entry);
        add(
            MenuSection::Edit,
            MenuEntry::new(t("Insert Sheet", "Insérer une feuille"), |a, _, cx| a.add_sheet(cx)).icon("plus"),
        );
        add(
            MenuSection::Edit,
            MenuEntry::new(t("Delete", "Supprimer"), move |a, _, cx| a.delete_sheet(index, cx))
                .icon("trash")
                .disabled(only),
        );
        add(
            MenuSection::Edit,
            MenuEntry::new(t("Rename", "Renommer"), move |a, window, cx| {
                a.begin_rename_sheet(index, window, cx)
            }),
        );
        add(
            MenuSection::Edit,
            MenuEntry::new(t("Duplicate", "Dupliquer"), move |a, _, cx| a.duplicate_sheet(index, cx)).icon("copy"),
        );
        let move_sheet = |label: &'static str, to: usize, disabled: bool| {
            MenuEntry::new(label, move |a: &mut ExcelApp, _: &mut Window, cx: &mut Context<ExcelApp>| {
                if to < a.wb.sheets.len() && a.may(Intent::MoveSheet, cx) {
                    a.wb.move_sheet(index, to);
                    a.views.swap(index, to);
                    a.sheet = to;
                    a.commit(t("Move Sheet", "Déplacer la feuille"), cx);
                }
            })
            .disabled(disabled)
        };
        add(
            MenuSection::Data,
            move_sheet(t("Move Left", "Déplacer vers la gauche"), index.saturating_sub(1), index == 0),
        );
        add(
            MenuSection::Data,
            move_sheet(t("Move Right", "Déplacer vers la droite"), index + 1, last),
        );
        add(
            MenuSection::Format,
            MenuEntry::new(t("Tab Color…", "Couleur d'onglet…"), move |a, _, cx| {
                a.popup = Some(Popup::new(PopupKind::Palette(ColorTarget::Tab), pos, 214.0));
                cx.notify();
            })
            .icon("fill"),
        );
        features::context_menu(self, &MenuTarget::SheetTab(index), &mut menu);
        self.popup = Some(Popup::new(PopupKind::Menu(menu.build()), pos, 230.0));
        cx.notify();
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = match &self.editing {
            None => t("Ready", "Prêt"),
            Some(e) if e.point.is_some() => t("Point", "Pointer"),
            Some(e) if e.mode == EditMode::Enter => crate::i18n::t_ctx("mode", "Enter", "Saisie"),
            Some(_) => crate::i18n::t_ctx("mode", "Edit", "Modifier"),
        };
        let stats = self.selection_stats();
        let locale = self.wb.locale;
        let fmt = |n: f64| opensheet_engine::format::format_general(n, locale, 12);
        let mut stat_items: Vec<AnyElement> = Vec::new();
        if stats.count > 1 {
            if let Some(avg) = stats.average() {
                stat_items.push(
                    div()
                        .child(crate::i18n::labelled(t("Average", "Moyenne"), fmt(avg)))
                        .into_any_element(),
                );
            }
            stat_items.push(
                div()
                    .child(crate::i18n::labelled(t("Count", "Nb (non vides)"), stats.count))
                    .into_any_element(),
            );
            if stats.numeric_count > 0 {
                stat_items.push(
                    div()
                        .child(crate::i18n::labelled(t("Sum", "Somme"), fmt(stats.sum)))
                        .into_any_element(),
                );
            }
        }
        let zoom = self.zoom;
        // Slider: 10 %..100 % on the left half, 100 %..400 % on the right.
        let slider_w = 100.0f32;
        let thumb_x = if zoom <= 1.0 {
            (zoom - 0.1) / 0.9 * (slider_w / 2.0)
        } else {
            slider_w / 2.0 + (zoom - 1.0) / 3.0 * (slider_w / 2.0)
        };
        let current_mode = self.view_mode();
        let view_btn = |name: &'static str, label: &'static str, mode: ViewMode, cx: &mut Context<Self>| {
            let on = mode == current_mode;
            div()
                .id(name)
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .when(!on, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(icon(name, 14.0, theme::TEXT_MUTED))
                .tooltip(tooltip(label))
                .on_click(cx.listener(move |this, _, _, cx| this.set_view_mode(mode, cx)))
        };
        // What the features have to say (records found by a filter, the
        // page count of the Page Layout view).
        let feature_items: Vec<AnyElement> = features::status_items(self)
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let action = item.on_click.clone();
                div()
                    .id(("status-item", i))
                    .child(item.text)
                    .when_some(item.tooltip, |d, tip| d.tooltip(tooltip(tip)))
                    .when_some(action, |d, action| {
                        d.rounded(px(3.))
                            .hover(|s| s.bg(c(theme::HOVER)))
                            .on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
                    })
                    .into_any_element()
            })
            .collect();
        let zoom_btn = |id: &'static str, icon_name: &'static str| {
            div()
                .id(id)
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(icon(icon_name, 12.0, theme::TEXT_MUTED))
        };
        div()
            .h(px(theme::STATUS_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .px_3()
            .gap_4()
            .bg(c(theme::CHROME_BG))
            .border_t_1()
            .border_color(c(theme::BORDER))
            .text_size(px(11.5))
            .text_color(c(theme::TEXT_MUTED))
            .child(div().child(mode))
            .when(self.wb.circular, |d| {
                d.child(div().child(t("Circular References", "Références circulaires")))
            })
            .children(feature_items)
            .when_some(self.status_message.clone(), |d, msg| d.child(div().child(msg)))
            .child(div().flex_1())
            .children(stat_items)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(view_btn("view-normal", t("Normal", "Normal"), ViewMode::Normal, cx))
                    .child(view_btn(
                        "view-layout",
                        crate::i18n::t_ctx("view", "Page Layout", "Mise en page"),
                        ViewMode::PageLayout,
                        cx,
                    ))
                    .child(view_btn(
                        "view-break",
                        t("Page Break Preview", "Aperçu des sauts de page"),
                        ViewMode::PageBreakPreview,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(zoom_btn("zoom-out", "minus").on_click(cx.listener(|this, _, _, cx| {
                        let z = ((this.zoom * 10.0).ceil() - 1.0) / 10.0;
                        this.set_zoom(z, cx)
                    })))
                    .child(
                        div()
                            .id("zoom-slider")
                            .relative()
                            .w(px(slider_w))
                            .h(px(16.))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(7.5))
                                    .w_full()
                                    .h(px(1.))
                                    .bg(c(theme::TEXT_MUTED)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(slider_w / 2.0))
                                    .top(px(4.))
                                    .w(px(1.))
                                    .h(px(8.))
                                    .bg(c(theme::TEXT_MUTED)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(thumb_x - 2.5))
                                    .top(px(2.))
                                    .w(px(5.))
                                    .h(px(12.))
                                    .rounded(px(1.))
                                    .bg(c(theme::TEXT_MUTED)),
                            )
                            .child(self.anchor("zoom-slider"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    let Some(b) = this.anchors.borrow().get("zoom-slider").copied() else {
                                        return;
                                    };
                                    let x = f32::from(ev.position.x - b.origin.x).clamp(0.0, slider_w);
                                    let half = slider_w / 2.0;
                                    let z = if x <= half {
                                        0.1 + x / half * 0.9
                                    } else {
                                        1.0 + (x - half) / half * 3.0
                                    };
                                    this.set_zoom((z * 20.0).round() / 20.0, cx);
                                }),
                            ),
                    )
                    .child(zoom_btn("zoom-in", "plus").on_click(cx.listener(|this, _, _, cx| {
                        let z = ((this.zoom * 10.0).floor() + 1.0) / 10.0;
                        this.set_zoom(z, cx)
                    })))
                    .child(
                        div()
                            .id("zoom-label")
                            .w(px(44.))
                            .text_right()
                            .child(format!("{:.0} %", zoom * 100.0))
                            .on_click(cx.listener(|this, _, _, cx| this.set_zoom(1.0, cx))),
                    ),
            )
    }

    fn render_find_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top(px(theme::TITLEBAR_H + theme::TABS_H + 6.0))
            .right(px(16.))
            .w(px(320.))
            .h(px(36.))
            .occlude()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(6.))
            .shadow_lg()
            .child(icon("search", 14.0, theme::TEXT_MUTED))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .child(self.find_input.clone()),
            )
            .child(
                div()
                    .id("find-next")
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(c(theme::ACCENT))
                    .text_color(c(0xFFFFFF))
                    .text_size(px(12.))
                    .child(t("Next", "Suivant"))
                    .on_click(cx.listener(|this, _, _, cx| this.find_next(cx))),
            )
            .child(
                div()
                    .id("find-close")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(icon("close", 12.0, theme::TEXT_MUTED))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_find(window, cx))),
            )
    }

    /// One level of an open menu.
    fn render_menu_panel(&self, entries: &[MenuEntry], level: usize, popup: &Popup, cx: &mut Context<Self>) -> AnyElement {
        let open = popup.open_path.get(level).copied();
        let mut items: Vec<AnyElement> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if entry.separator {
                items.push(
                    div()
                        .h(px(1.))
                        .my_1()
                        .mx_2()
                        .bg(c(theme::BORDER))
                        .into_any_element(),
                );
                continue;
            }
            let action = entry.action.clone();
            let disabled = entry.disabled;
            let has_submenu = entry.submenu.is_some() && !disabled;
            let fg = if disabled { theme::TEXT_DISABLED } else { theme::TEXT };
            let geometry = popup.geometry.clone();
            items.push(
                div()
                    .id(("menu-item", level * 1000 + i))
                    .relative()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .mx_1()
                    .rounded(px(4.))
                    .text_size(px(12.5))
                    .text_color(c(fg))
                    .when(open == Some(i), |d| d.bg(c(theme::HOVER)))
                    .when(!disabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                    .child(
                        div()
                            .w(px(18.))
                            .flex()
                            .justify_center()
                            .when(entry.checked, |d| d.child(icon("check", 14.0, theme::ACCENT)))
                            .when_some(entry.icon.filter(|_| !entry.checked), |d, name| {
                                d.child(icon(name, 14.0, if disabled { theme::TEXT_DISABLED } else { theme::TEXT_MUTED }))
                            }),
                    )
                    .child(div().flex_1().truncate().child(entry.label.clone()))
                    .when_some(entry.shortcut, |d, s| {
                        d.child(div().text_color(c(theme::TEXT_MUTED)).text_size(px(11.5)).child(s))
                    })
                    .when(entry.submenu.is_some(), |d| {
                        d.child(icon("chevron-right", 10.0, if disabled { theme::TEXT_DISABLED } else { theme::TEXT_MUTED }))
                            // Where its submenu hangs.
                            .child(
                                canvas(
                                    move |bounds, _, _| {
                                        geometry.borrow_mut().items.insert((level, i), bounds);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                    })
                    // Resting on an entry opens its submenu and closes
                    // those of its siblings.
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        let Some(popup) = &mut this.popup else {
                            return;
                        };
                        let wanted = has_submenu.then_some(i);
                        if popup.open_path.get(level).copied() != wanted {
                            popup.open_path.truncate(level);
                            popup.open_path.extend(wanted);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if disabled || has_submenu {
                            return;
                        }
                        this.popup = None;
                        if let Some(action) = &action {
                            action(this, window, cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        div().py_1().children(items).into_any_element()
    }

    /// A press outside a panel of the pop-up closes it, unless it lands on
    /// another of its panels.
    fn popup_press_outside(&mut self, position: gpui::Point<gpui::Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = &self.popup else {
            return;
        };
        if popup.geometry.borrow().panels.iter().any(|panel| panel.contains(&position)) {
            return;
        }
        // A press on the control the pop-up hangs from is left to that
        // control: its click closes the pop-up (see `open_menu`).
        let on_anchor = popup
            .anchor
            .and_then(|anchor| self.anchors.borrow().get(anchor).copied())
            .is_some_and(|bounds| bounds.contains(&position));
        if on_anchor {
            return;
        }
        // Remember what was closed so the click that follows on a control
        // sharing this anchor (the arrow beside a button) does not reopen
        // it.
        self.popup_dismissed = popup.anchor;
        self.close_popup(window, cx);
    }

    /// The frame of a floating panel that records where it was painted.
    fn popup_panel(&self, level: usize, width: f32, popup: &Popup, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let geometry = popup.geometry.clone();
        div()
            .id(("popup", level))
            .relative()
            .occlude()
            .w(px(width))
            .max_h(px(popup.max_height))
            .overflow_y_scroll()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(6.))
            .shadow_lg()
            .text_color(c(theme::TEXT))
            .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.popup_press_outside(ev.position, window, cx)
            }))
            .child(
                canvas(
                    move |bounds, _, _| {
                        let panels = &mut geometry.borrow_mut().panels;
                        if panels.len() <= level {
                            panels.resize(level + 1, Default::default());
                        }
                        panels[level] = bounds;
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }

    /// The open menu or palette. Drawn after everything else of the window.
    fn render_popup(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let popup = self.popup.as_ref()?;
        let mut panels: Vec<AnyElement> = Vec::new();
        match &popup.kind {
            PopupKind::View(_) => return None,
            PopupKind::Menu(entries) => {
                // The menu, then one panel per open submenu.
                popup.geometry.borrow_mut().panels.truncate(popup.open_path.len() + 1);
                let mut level_entries: &[MenuEntry] = entries;
                let mut position = popup.pos;
                let mut width = popup.width;
                for level in 0..=popup.open_path.len() {
                    panels.push(
                        anchored()
                            .position(position)
                            .snap_to_window_with_margin(px(6.))
                            .child(
                                self.popup_panel(level, width, popup, cx)
                                    .child(self.render_menu_panel(level_entries, level, popup, cx)),
                            )
                            .into_any_element(),
                    );
                    let Some(index) = popup.open_path.get(level).copied() else {
                        break;
                    };
                    let Some(children) = level_entries.get(index).and_then(|e| e.submenu.as_deref()) else {
                        break;
                    };
                    let Some(item) = popup.geometry.borrow().items.get(&(level, index)).copied() else {
                        break;
                    };
                    position = gpui::point(item.right() + px(2.), item.top() - px(5.));
                    // Wide enough for the longest label.
                    let longest = children.iter().map(|e| e.label.chars().count()).max().unwrap_or(0);
                    width = (longest as f32 * 7.2 + 70.0).clamp(170.0, 380.0);
                    level_entries = children;
                }
            }
            PopupKind::Palette(target) => {
                let target = *target;
                let swatch = move |color: u32, cx: &mut Context<Self>| {
                    div()
                        .id(("swatch", color as usize))
                        .size(px(18.))
                        .bg(c(color))
                        .border_1()
                        .border_color(c(0xD0D0D0))
                        .hover(|s| s.border_color(c(0xF2994A)))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick_color(target, Some(Color(color)), cx)))
                };
                let label = |text: &'static str| {
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c(theme::TEXT_MUTED))
                        .py_1()
                        .child(text)
                };
                let theme_rows: Vec<AnyElement> = theme::palette_rows()
                    .into_iter()
                    .map(|row| {
                        div()
                            .flex()
                            .gap(px(2.))
                            .children(row.into_iter().map(|col| swatch(col, cx)))
                            .into_any_element()
                    })
                    .collect();
                let body = div()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .id("palette-none")
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_1()
                            .rounded(px(4.))
                            .hover(|s| s.bg(c(theme::HOVER)))
                            .text_size(px(12.5))
                            .child(
                                div()
                                    .size(px(16.))
                                    .border_1()
                                    .border_color(c(theme::BORDER_STRONG))
                                    .bg(c(0xFFFFFF)),
                            )
                            .child(match target {
                                ColorTarget::Font => t("Automatic", "Automatique"),
                                ColorTarget::Fill => t("No Fill", "Aucun remplissage"),
                                ColorTarget::Tab => t("No Color", "Aucune couleur"),
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_color(target, None, cx))),
                    )
                    .child(label(t("Theme Colors", "Couleurs du thème")))
                    .children(theme_rows)
                    .child(label(t("Standard Colors", "Couleurs standard")))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .children(theme::PALETTE_STANDARD.iter().map(|col| swatch(*col, cx))),
                    );
                panels.push(
                    anchored()
                        .position(popup.pos)
                        .snap_to_window_with_margin(px(6.))
                        .child(self.popup_panel(0, popup.width, popup, cx).child(body))
                        .into_any_element(),
                );
            }
        }
        Some(deferred(div().children(panels)).with_priority(1).into_any_element())
    }

    /// The drop-down of a feature (`open_popup_view`). It is not deferred
    /// like the menus are: the widgets inside it may open lists of their
    /// own, and those are.
    fn render_popup_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let popup = self.popup.as_ref()?;
        let PopupKind::View(view) = &popup.kind else {
            return None;
        };
        Some(
            anchored()
                .position(popup.pos)
                .snap_to_window_with_margin(px(6.))
                .child(
                    div()
                        .id("popup-view")
                        .occlude()
                        .bg(c(0xFFFFFF))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .rounded(px(6.))
                        .shadow_lg()
                        .text_color(c(theme::TEXT))
                        // Tab walks its controls, Escape closes it.
                        .key_context(crate::widgets::CONTEXT)
                        .on_action(|_: &crate::widgets::FocusNext, window, _| window.focus_next())
                        .on_action(|_: &crate::widgets::FocusPrev, window, _| window.focus_prev())
                        .on_action(|_: &crate::text_input::Tab, window, _| window.focus_next())
                        .on_action(|_: &crate::text_input::ShiftTab, window, _| window.focus_prev())
                        .on_action(cx.listener(|this, _: &crate::widgets::Dismiss, window, cx| this.close_popup(window, cx)))
                        .on_action(cx.listener(|this, _: &crate::text_input::Escape, window, cx| this.close_popup(window, cx)))
                        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_popup(window, cx)))
                        .child(view.clone()),
                )
                .into_any_element(),
        )
    }
}

impl Render for ExcelApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&format!("{} — OpenSheet", self.title));
        let titlebar = self.render_titlebar(window, cx).into_any_element();
        let tabs = self.render_ribbon_tabs(cx).into_any_element();
        let ribbon = self.render_ribbon(window, cx).into_any_element();
        let formula_bar = self
            .show_formula_bar
            .then(|| self.render_formula_bar(cx).into_any_element());
        let grid = self.render_grid(window, cx).into_any_element();
        let pane = self.render_panes(cx);
        let sheet_tabs = self.render_sheet_tabs(cx).into_any_element();
        let status = self.render_status_bar(cx).into_any_element();
        let find = self.show_find.then(|| self.render_find_bar(cx).into_any_element());
        let popup = self.render_popup(cx);
        let popup_view = self.render_popup_view(cx);
        let modals = self.render_modals(window, cx);
        // Behind a dialog the workbook is deaf: its shortcuts and the menu
        // bar do nothing until the dialog is closed.
        let modal = self.modal_open();

        div()
            .id("workbook")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::CHROME_BG))
            .text_color(c(theme::TEXT))
            .text_size(px(12.))
            .key_context(if modal { "WorkbookBehindDialog" } else { "Workbook" })
            // A new press starts with a clean slate: this capture listener runs
            // before the pop-up's own "mouse down outside" handler, so the
            // flag only ever describes the press in progress.
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.popup_dismissed = None))
            // Runs after the clicked control's own handler (bubble order).
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.popup_dismissed = None;
                    this.pane_resize = None;
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.pane_resize_move(ev, cx)))
            .when(!modal, |d| {
                d.on_action(cx.listener(Self::on_undo))
                    .on_action(cx.listener(Self::on_redo))
                    .on_action(cx.listener(Self::on_bold))
                    .on_action(cx.listener(Self::on_italic))
                    .on_action(cx.listener(Self::on_underline))
                    .on_action(cx.listener(Self::on_find))
                    .on_action(cx.listener(Self::on_zoom_in))
                    .on_action(cx.listener(Self::on_zoom_out))
                    .on_action(cx.listener(Self::on_zoom_reset))
                    .on_action(cx.listener(Self::on_next_sheet))
                    .on_action(cx.listener(Self::on_prev_sheet))
                    .on_action(cx.listener(Self::on_cancel))
                    .on_action(cx.listener(Self::on_format_cells))
                    .on_action(cx.listener(Self::on_print))
                    .on_action(cx.listener(Self::on_toggle_filter))
                    .on_action(cx.listener(Self::on_insert_table))
                    .on_action(cx.listener(Self::on_new_note))
                    .on_action(cx.listener(Self::on_new_comment))
                    .on_action(cx.listener(Self::on_name_manager))
                    .on_action(cx.listener(Self::on_insert_link))
                    .on_action(cx.listener(|this, _: &Save, window, cx| this.save(false, window, cx)))
                    .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save(true, window, cx)))
                    .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
                    .on_action(cx.listener(|this, _: &NewWorkbook, window, cx| this.new_workbook(window, cx)))
            })
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.request_quit(window, cx)))
            // A press reported outside the window never comes from a real
            // pointer: automation tools send one after each click. Painted
            // first, this listener swallows it before a pop-up, painted later,
            // takes it for a click elsewhere and closes.
            .child(
                canvas(
                    |_, _, _| {},
                    |_, _, window, _| {
                        window.on_mouse_event(|ev: &MouseDownEvent, phase, _, cx| {
                            let outside = ev.position.x < px(0.) || ev.position.y < px(0.);
                            if phase == DispatchPhase::Capture && outside {
                                cx.stop_propagation();
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
            .child(titlebar)
            .child(tabs)
            .child(ribbon)
            .children(formula_bar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .flex()
                    .child(div().flex_1().min_w(px(0.)).h_full().flex().flex_col().child(grid))
                    .children(pane),
            )
            .child(sheet_tabs)
            .child(status)
            .children(find)
            .children(popup)
            .children(popup_view)
            .children(modals)
    }
}
