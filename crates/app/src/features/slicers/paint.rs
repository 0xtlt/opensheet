//! Drawing slicers and timelines on the grid, and registering the zone
//! that makes their parts answer the mouse.

use super::logic::{self, Box4, TOOL};
use super::{SlicerUi, SlicerZone, TimelineZone, first_row, items_of, multi_select};
use crate::features::Painter;
use crate::grid::Draw;
use crate::theme::c;
use gpui::{Bounds, Pixels, px, size};
use opensheet_engine::slicer::{all_periods, periods, span_label};
use std::rc::Rc;

/// Maps the boxes of a layout (pixels at 100 % from the corner of the
/// object) to the grid element.
#[derive(Clone, Copy)]
struct Frame {
    x: f32,
    y: f32,
    z: f32,
}

impl Frame {
    fn at(&self, b: Box4) -> Box4 {
        (self.x + b.0 * self.z, self.y + b.1 * self.z, self.x + b.2 * self.z, self.y + b.3 * self.z)
    }
}

/// The part of `b` inside `clip`; `None` when nothing of it is.
fn cut(b: Box4, clip: Box4) -> Option<Box4> {
    let out = (b.0.max(clip.0), b.1.max(clip.1), b.2.min(clip.2), b.3.min(clip.3));
    (out.0 < out.2 && out.1 < out.3).then_some(out)
}

fn bounds(p: &Painter, b: Box4) -> Bounds<Pixels> {
    Bounds::new(p.layout.to_window(b.0, b.1), size(px(b.2 - b.0), px(b.3 - b.1)))
}

/// How a line of text sits in its box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// One line of text in a box, cut off at `clip`.
fn text_in(p: &mut Painter, text: &str, b: Box4, clip: Box4, font_px: f32, color: u32, bold: bool, align: Align) {
    let Some(clip) = cut(b, clip).and_then(|inner| cut(inner, p.clip)) else {
        return;
    };
    let line = p.shape(text, font_px, c(color), bold);
    let width = f32::from(line.width);
    let x = match align {
        Align::Left => b.0,
        Align::Centre => b.0 + ((b.2 - b.0) - width).max(0.0) / 2.0,
        Align::Right => (b.2 - width).max(b.0),
    };
    let line_height = (font_px * 1.25).round();
    let y = b.1 + ((b.3 - b.1) - line_height) / 2.0;
    let clip = bounds(p, clip);
    p.push(Draw::Text {
        line,
        origin: p.layout.to_window(x.round(), y.round()),
        line_height: px(line_height),
        clip,
    });
}

fn fill(p: &mut Painter, b: Box4, clip: Box4, color: u32) {
    if let Some(b) = cut(b, clip) {
        p.rect(b.0, b.1, b.2, b.3, c(color));
    }
}

/// The funnel with its cross of "Clear Filter", lit when there is a
/// filter to clear.
fn clear_icon(p: &mut Painter, b: Box4, z: f32, lit: bool, ink: u32) {
    let (x, y) = (b.0, b.1);
    let u = (b.2 - b.0) / TOOL;
    let grey = if lit { ink } else { 0xB0B0B0 };
    let funnel = [
        (x + 3.0 * u, y + 4.0 * u),
        (x + 14.0 * u, y + 4.0 * u),
        (x + 10.0 * u, y + 9.5 * u),
        (x + 10.0 * u, y + 14.0 * u),
        (x + 7.0 * u, y + 16.0 * u),
        (x + 7.0 * u, y + 9.5 * u),
    ];
    let mut outline = funnel.to_vec();
    outline.push(funnel[0]);
    p.polyline(&outline, 1.2 * z.max(0.75), c(grey));
    let cross = if lit { 0xC8102E } else { 0xB0B0B0 };
    p.polyline(&[(x + 12.5 * u, y + 11.5 * u), (x + 17.5 * u, y + 16.5 * u)], 1.6 * z.max(0.75), c(cross));
    p.polyline(&[(x + 17.5 * u, y + 11.5 * u), (x + 12.5 * u, y + 16.5 * u)], 1.6 * z.max(0.75), c(cross));
}

/// Three ticked lines: "Multi-Select".
fn multi_icon(p: &mut Painter, b: Box4, z: f32, ink: u32) {
    let u = (b.2 - b.0) / TOOL;
    for k in 0..3 {
        let y = b.1 + (5.0 + 5.0 * k as f32) * u;
        p.polyline(&[(b.0 + 3.0 * u, y), (b.0 + 4.5 * u, y + 1.5 * u), (b.0 + 7.0 * u, y - 1.5 * u)], 1.2 * z.max(0.75), c(ink));
        p.rect(b.0 + 9.0 * u, y - 0.5 * u, b.0 + 17.0 * u, y + 0.5 * u, c(ink));
    }
}

/// A scroll bar: its track and its thumb.
fn scroll_bar(p: &mut Painter, frame: Frame, track: Box4, thumb: Box4) {
    let (t, h) = (frame.at(track), frame.at(thumb));
    p.rect(t.0, t.1, t.2, t.3, c(0xF0F0F0));
    p.rounded(h.0 + 1.0, h.1 + 1.0, h.2 - 1.0, h.3 - 1.0, 3.0, c(0xC1C1C1), None);
}

pub(super) fn slicer(p: &mut Painter, id: u32, rect: Box4) {
    let Some((_, slicer)) = p.app.wb.slicer(id) else {
        return;
    };
    let z = p.zoom();
    let frame = Frame { x: rect.0, y: rect.1, z };
    let look = logic::slicer_look(p.app.wb.theme(), &slicer.style);
    let items = items_of(p.app, id);
    let layout = logic::slicer_layout(
        slicer.size,
        slicer.show_header,
        slicer.columns,
        slicer.button_height,
        items.len(),
        first_row(p.app, slicer),
    );
    // What a press in progress selects shows before it is applied.
    let pending = p.app.ext::<SlicerUi>().and_then(|ui| ui.pending_items.as_ref()).filter(|(s, _)| *s == id);
    let selected = |index: usize| match pending {
        Some((_, chosen)) => chosen.is_empty() || chosen.contains(&items[index].x),
        None => items[index].selected,
    };
    let font = 12.0 * z;
    p.rect(rect.0, rect.1, rect.2, rect.3, c(look.background));
    if let Some(header) = layout.header {
        let h = frame.at(header);
        if let Some(color) = look.header_fill {
            p.rect(h.0, h.1, h.2, h.3, c(color));
        }
        let rule = frame.at((logic::PAD, header.3 - 1.0, header.2 - logic::PAD, header.3));
        p.rect(rule.0, rule.1, rule.2, rule.1 + z.max(1.0), c(look.header_rule));
        let tools = layout.multi.map_or(header.2, |b| b.0);
        let caption = frame.at((logic::PAD, header.1, tools - 4.0, header.3));
        text_in(p, &slicer.caption, caption, rect, font, look.header_text, true, Align::Left);
        if let Some(b) = layout.multi {
            let b = frame.at(b);
            if multi_select(p.app, id) {
                p.rounded(b.0, b.1, b.2, b.3, 3.0, crate::theme::ca(look.header_text, 0.18), None);
            }
            multi_icon(p, b, z, look.header_text);
        }
        if let Some(b) = layout.clear {
            let filtered = p.app.wb.slicer_filtered(id);
            clear_icon(p, frame.at(b), z, filtered, look.header_text);
        }
    }
    let body = frame.at(layout.body);
    for (index, button) in &layout.buttons {
        let item = &items[*index];
        // "Visually indicate items with no data" off: they look like the rest.
        let tile = look.tile(selected(*index), item.has_data || !slicer.indicate_no_data);
        let b = frame.at(*button);
        fill(p, b, body, tile.border);
        fill(p, (b.0 + 1.0, b.1 + 1.0, b.2 - 1.0, b.3 - 1.0), body, tile.fill);
        let label = (b.0 + 6.0 * z, b.1, b.2 - 4.0 * z, b.3);
        text_in(p, &item.label, label, body, font, tile.text, false, Align::Left);
    }
    if let Some((track, thumb)) = layout.scroll {
        scroll_bar(p, frame, track, thumb);
    }
    p.frame(rect.0, rect.1, rect.2, rect.3, 1.0, c(look.border));
    // The handles on the frame stay those of the object.
    p.zone(
        (rect.0 + 5.0, rect.1 + 5.0, rect.2 - 5.0, rect.3 - 5.0),
        SlicerZone {
            id,
            origin: (rect.0, rect.1),
            zoom: z,
            layout,
            items,
        },
    );
}

pub(super) fn timeline(p: &mut Painter, id: u32, rect: Box4) {
    let Some((_, timeline)) = p.app.wb.timeline(id) else {
        return;
    };
    let z = p.zoom();
    let frame = Frame { x: rect.0, y: rect.1, z };
    let look = logic::timeline_look(p.app.wb.theme(), &timeline.style);
    let locale = p.app.wb.locale;
    let bounds = p.app.wb.timeline_bounds(id);
    let band: Vec<_> = bounds.map(|(start, end)| periods(timeline.level, start, end, locale)).unwrap_or_default();
    let selection = p.app.wb.timeline_selection(id);
    let ui = p.app.ext::<SlicerUi>();
    let pending = ui.and_then(|ui| ui.pending_span).filter(|(t, ..)| *t == id).map(|(_, a, b)| (a, b));
    let selected = pending.or_else(|| logic::selected_periods(&band, selection));
    let mut shape = logic::TimelineShape {
        size: timeline.size,
        header: timeline.show_header,
        selection_label: timeline.show_selection_label,
        time_level: timeline.show_time_level,
        scrollbar: timeline.show_scrollbar,
        level: timeline.level,
        periods: band.len(),
        scroll: 0.0,
    };
    // Until the user scrolls it, the band starts where the selection is,
    // or where the file left it.
    shape.scroll = match ui.and_then(|ui| ui.band_scroll.get(&id).copied()) {
        Some(scroll) => scroll,
        None => {
            let unscrolled = logic::timeline_layout(&shape);
            let first = selected.map(|(first, _)| first).or_else(|| {
                let day = timeline.scroll?;
                band.iter().position(|period| period.end >= day)
            });
            first.map_or(0.0, |first| logic::scroll_to_show(&unscrolled, first))
        }
    };
    let layout = logic::timeline_layout(&shape);
    let font = 12.0 * z;
    p.rect(rect.0, rect.1, rect.2, rect.3, c(look.background));
    if let Some(header) = layout.header {
        let caption = frame.at((logic::TL_PAD, header.1, layout.clear.map_or(header.2, |b| b.0) - 4.0, header.3));
        text_in(p, &timeline.caption, caption, rect, 13.0 * z, look.header_text, true, Align::Left);
        if let Some(b) = layout.clear {
            clear_icon(p, frame.at(b), z, selection.is_some(), look.header_text);
        }
    }
    if let Some(b) = layout.selection_label {
        let days = pending.and_then(|(a, b)| logic::span_days(&band, a, b)).or(selection);
        let text = match days {
            Some((start, end)) => span_label(timeline.level, start, end, locale),
            None => all_periods(locale).to_string(),
        };
        text_in(p, &text, frame.at(b), rect, font, look.label_text, false, Align::Left);
    }
    if let Some(b) = layout.level {
        let text = format!("{} \u{25BE}", timeline.level.label(locale));
        text_in(p, &text, frame.at(b), rect, 11.0 * z, look.label_text, false, Align::Right);
    }
    let clip = frame.at(layout.band);
    let small = 10.5 * z;
    for (index, period) in band.iter().enumerate() {
        let x0 = layout.period_x(index);
        if x0 + layout.tile < layout.band.0 || x0 > layout.band.2 {
            continue;
        }
        if let Some(group) = &period.group {
            let b = frame.at((x0 + 2.0, layout.groups_y, x0 + 400.0, layout.groups_y + logic::TL_TEXT_H));
            text_in(p, group, b, clip, small, look.period_text, true, Align::Left);
        }
        let name = frame.at((x0, layout.names_y, x0 + layout.tile, layout.names_y + logic::TL_TEXT_H));
        text_in(p, &period.label, name, clip, small, look.period_text, false, Align::Centre);
        let on = selected.is_some_and(|(first, last)| index >= first && index <= last);
        let bar = frame.at((x0 + 1.0, layout.bar.0, x0 + layout.tile - 1.0, layout.bar.1));
        fill(p, bar, clip, if on { look.selected } else { look.unselected });
    }
    // The handles at the two ends of the selected span.
    if let Some((first, last)) = selected {
        for x in [layout.period_x(first), layout.period_x(last + 1)] {
            let handle = frame.at((x - 3.0, layout.bar.0 - 3.0, x + 3.0, layout.bar.1 + 3.0));
            if let Some(b) = cut(handle, (clip.0 - 3.0 * z, clip.1, clip.2 + 3.0 * z, clip.3 + 4.0 * z)) {
                p.rounded(b.0, b.1, b.2, b.3, 2.0, c(look.handle), None);
            }
        }
    }
    if let Some((track, thumb)) = layout.scrollbar {
        scroll_bar(p, frame, track, thumb);
    }
    p.frame(rect.0, rect.1, rect.2, rect.3, 1.0, c(look.border));
    p.zone(
        (rect.0 + 5.0, rect.1 + 5.0, rect.2 - 5.0, rect.3 - 5.0),
        TimelineZone {
            id,
            origin: (rect.0, rect.1),
            zoom: z,
            layout,
            periods: Rc::new(band),
            selected,
        },
    );
}
