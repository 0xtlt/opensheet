//! The drop-down galleries of charts: chart types on the Insert tab, Quick
//! Layout, Change Colors and Chart Styles. Every item is a small picture
//! (a chart drawn by the real layout, or a row of colours) under the
//! heading of its section, as in Excel.

use crate::app::{ExcelApp, MenuAction};
use crate::chart_view;
use crate::theme::{self, c};
use crate::view::tooltip;
use gpui::{Context, FontWeight, Render, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::chart::{Chart, ChartData};
use opensheet_engine::style::Color;
use opensheet_engine::Locale;
use std::rc::Rc;

/// What an item shows.
pub(crate) enum Picture {
    Chart(Rc<Chart>, Rc<ChartData>),
    Colors(Vec<Color>),
}

pub(crate) struct Item {
    pub picture: Picture,
    pub tip: SharedString,
    pub action: MenuAction,
}

impl Item {
    pub fn new(picture: Picture, tip: impl Into<SharedString>, action: impl Fn(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static) -> Item {
        Item {
            picture,
            tip: tip.into(),
            action: Rc::new(action),
        }
    }
}

pub(crate) struct Section {
    pub title: SharedString,
    pub items: Vec<Item>,
}

pub(crate) struct Gallery {
    app: WeakEntity<ExcelApp>,
    sections: Vec<Section>,
    /// Entries under the pictures ("More Column Charts…").
    footer: Vec<(SharedString, MenuAction)>,
    /// Size of a picture and the zoom charts are drawn at.
    thumb: (f32, f32),
    zoom: f32,
    columns: usize,
    font: SharedString,
    locale: &'static Locale,
}

impl Gallery {
    pub fn new(app: &ExcelApp, cx: &mut Context<ExcelApp>, sections: Vec<Section>) -> Gallery {
        Gallery {
            app: cx.weak_entity(),
            sections,
            footer: Vec::new(),
            thumb: (52.0, 40.0),
            zoom: 0.4,
            columns: 4,
            font: app.chart_font(),
            locale: app.wb.locale,
        }
    }

    pub fn thumb(mut self, width: f32, height: f32, zoom: f32) -> Gallery {
        self.thumb = (width, height);
        self.zoom = zoom;
        self
    }

    pub fn columns(mut self, columns: usize) -> Gallery {
        self.columns = columns.max(1);
        self
    }

    pub fn footer(mut self, label: impl Into<SharedString>, action: impl Fn(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static) -> Gallery {
        self.footer.push((label.into(), Rc::new(action)));
        self
    }

    /// Close the drop-down, then do what was chosen.
    fn run(&self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                action(app, window, cx);
            })
            .ok();
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        const PAD: f32 = 4.0;
        let (tw, th) = self.thumb;
        let cell_w = tw + 2.0 * PAD + 2.0;
        let width = cell_w * self.columns as f32 + 16.0;
        let mut body = div().w(px(width)).py(px(4.)).flex().flex_col().text_size(px(12.));
        let mut id = 0usize;
        for section in &self.sections {
            if !section.title.is_empty() {
                body = body.child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .bg(c(theme::CHROME_BG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c(theme::TEXT_MUTED))
                        .child(section.title.clone()),
                );
            }
            let mut grid = div().px(px(8.)).py(px(4.)).flex().flex_wrap();
            for item in &section.items {
                id += 1;
                let action = item.action.clone();
                let picture = match &item.picture {
                    Picture::Chart(chart, data) => chart_view::chart_canvas(
                        chart.clone(),
                        data.clone(),
                        tw,
                        th,
                        self.zoom,
                        self.font.clone(),
                        self.locale,
                    )
                    .into_any_element(),
                    Picture::Colors(colors) => div()
                        .w(px(tw))
                        .h(px(th))
                        .flex()
                        .items_center()
                        .children(colors.iter().map(|color| div().flex_1().h_full().bg(c(color.0))))
                        .into_any_element(),
                };
                grid = grid.child(
                    div()
                        .id(("gallery-item", id))
                        .w(px(cell_w))
                        .p(px(PAD))
                        .border_1()
                        .rounded(px(3.))
                        .border_color(gpui::transparent_black())
                        .hover(|s| s.bg(c(theme::HOVER)).border_color(c(theme::BORDER_STRONG)))
                        .child(picture)
                        .tooltip(tooltip(item.tip.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| this.run(action.clone(), window, cx))),
                );
            }
            body = body.child(grid);
        }
        if !self.footer.is_empty() {
            body = body.child(div().h(px(1.)).my(px(3.)).bg(c(theme::BORDER)));
        }
        for (i, (label, action)) in self.footer.iter().enumerate() {
            let action = action.clone();
            body = body.child(
                div()
                    .id(("gallery-footer", i))
                    .h(px(26.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(label.clone())
                    .on_click(cx.listener(move |this, _, window, cx| this.run(action.clone(), window, cx))),
            );
        }
        body
    }
}
