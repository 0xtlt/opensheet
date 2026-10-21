//! How the form controls are drawn on the grid — as Excel draws its own:
//! flat, in the greys of the system — and where the parts of each are
//! (the arrows of a spin button, the thumb of a scroll bar, the lines of
//! a list), which is what a click is told from.

use super::{ControlZone, list_top, pressed_part};
use crate::features::Painter;
use crate::grid::Draw;
use crate::theme::c;
use opensheet_engine::control::{CheckState, ControlKind, FormControl, stepped};

const FACE: u32 = 0xF0F0F0;
const FACE_DOWN: u32 = 0xD4D4D4;
const EDGE: u32 = 0x8A8A8A;
const INK: u32 = 0x1F1F1F;
const FIELD: u32 = 0xFFFFFF;
const CHOSEN: u32 = 0x0A64D2;
/// Height of a line of a list, and the size of the text of a control, in
/// pixels at 100 %.
const LINE: f32 = 16.0;
const TEXT: f32 = 12.0;

/// What a point of a control is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Nothing that answers a click (a label, a group box, the empty
    /// part of a list).
    Nothing,
    /// The whole of a button, a check box or an option button.
    Face,
    /// A combo box: a press opens its list.
    Drop,
    /// A line of a list box (1 = the first).
    Line(u32),
    /// The arrow at the top (or the left) of a spin button or scroll bar,
    /// and the one at the other end.
    Less,
    More,
    /// The track of a scroll bar before its thumb, and after it.
    PageLess,
    PageMore,
    Thumb,
}

/// Where the parts of a control are, for the box it is drawn in.
pub(crate) struct Geometry {
    kind: ControlKind,
    rect: (f32, f32, f32, f32),
    zoom: f32,
    /// A scroll bar that lies down.
    horizontal: bool,
    /// The lines of a list, and the first one shown.
    lines: u32,
    top: u32,
    /// Where the value of a scroll bar is between its ends, 0 to 1, and
    /// how much of its range a page is.
    fraction: f32,
    page: f32,
}

impl Geometry {
    /// `control`: the control as it shows now (its state resolved against
    /// the cells).
    pub(crate) fn of(
        control: &FormControl,
        rect: (f32, f32, f32, f32),
        zoom: f32,
        lines: u32,
        top: u32,
    ) -> Geometry {
        let (lo, hi) = (
            control.min.min(control.max) as f32,
            control.min.max(control.max) as f32,
        );
        let span = (hi - lo).max(1.0);
        Geometry {
            kind: control.kind,
            rect,
            zoom,
            horizontal: control.kind == ControlKind::ScrollBar && control.horizontal,
            lines,
            top,
            fraction: ((control.value as f32 - lo) / span).clamp(0.0, 1.0),
            page: (control.page.max(1) as f32 / (span + control.page.max(1) as f32))
                .clamp(0.05, 1.0),
        }
    }

    /// Length and thickness of a bar.
    fn bar(&self) -> (f32, f32) {
        let (w, h) = (self.rect.2 - self.rect.0, self.rect.3 - self.rect.1);
        if self.horizontal { (w, h) } else { (h, w) }
    }

    /// The length of an arrow button of a bar.
    fn arrow(&self) -> f32 {
        let (length, thickness) = self.bar();
        thickness.min(length / 3.0)
    }

    fn thumb_length(&self) -> f32 {
        let track = self.bar().0 - 2.0 * self.arrow();
        (track * self.page).max(8.0 * self.zoom).min(track)
    }

    /// Where the thumb of a scroll bar begins, along the bar.
    pub(crate) fn thumb_start(&self) -> f32 {
        let track = self.bar().0 - 2.0 * self.arrow();
        self.arrow() + (track - self.thumb_length()) * self.fraction
    }

    /// How far along its bar a point of the grid element is.
    pub(crate) fn along(&self, x: f32, y: f32) -> f32 {
        if self.horizontal {
            x - self.rect.0
        } else {
            y - self.rect.1
        }
    }

    /// The value of a scroll bar whose thumb begins at `start` along it.
    pub(crate) fn value_at(&self, control: &FormControl, start: f32) -> i32 {
        let room = self.bar().0 - 2.0 * self.arrow() - self.thumb_length();
        let fraction = if room > 0.0 {
            ((start - self.arrow()) / room).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (lo, hi) = (control.min.min(control.max), control.min.max(control.max));
        stepped(
            control,
            lo + (fraction * (hi - lo) as f32).round() as i32,
            0,
        )
    }

    /// How many lines of a list box show at once.
    pub(crate) fn lines_shown(&self) -> u32 {
        (((self.rect.3 - self.rect.1 - 2.0) / (LINE * self.zoom)).floor() as u32).max(1)
    }

    pub(crate) fn part_at(&self, x: f32, y: f32) -> Part {
        match self.kind {
            ControlKind::Button | ControlKind::CheckBox | ControlKind::OptionButton => Part::Face,
            ControlKind::ComboBox => Part::Drop,
            ControlKind::GroupBox | ControlKind::Label => Part::Nothing,
            ControlKind::ListBox => {
                let row = ((y - self.rect.1 - 1.0) / (LINE * self.zoom)).floor();
                let line = self.top + row.max(0.0) as u32 + 1;
                if row >= 0.0 && line <= self.lines {
                    Part::Line(line)
                } else {
                    Part::Nothing
                }
            }
            ControlKind::SpinButton => {
                if y < (self.rect.1 + self.rect.3) / 2.0 {
                    Part::Less
                } else {
                    Part::More
                }
            }
            ControlKind::ScrollBar => {
                let (at, length) = (self.along(x, y), self.bar().0);
                let (thumb, arrow) = (self.thumb_start(), self.arrow());
                if at < arrow {
                    Part::Less
                } else if at >= length - arrow {
                    Part::More
                } else if at < thumb {
                    Part::PageLess
                } else if at >= thumb + self.thumb_length() {
                    Part::PageMore
                } else {
                    Part::Thumb
                }
            }
        }
    }
}

/// A small triangle pointing up, down, left or right (0 to 3).
fn triangle(p: &mut Painter, cx: f32, cy: f32, size: f32, direction: u8) {
    let s = size;
    let points = match direction {
        0 => [
            (cx - s, cy + s * 0.6),
            (cx + s, cy + s * 0.6),
            (cx, cy - s * 0.6),
        ],
        1 => [
            (cx - s, cy - s * 0.6),
            (cx + s, cy - s * 0.6),
            (cx, cy + s * 0.6),
        ],
        2 => [
            (cx + s * 0.6, cy - s),
            (cx + s * 0.6, cy + s),
            (cx - s * 0.6, cy),
        ],
        _ => [
            (cx - s * 0.6, cy - s),
            (cx - s * 0.6, cy + s),
            (cx + s * 0.6, cy),
        ],
    };
    p.polygon(&points, c(INK));
}

/// A raised button face.
fn face(p: &mut Painter, x0: f32, y0: f32, x1: f32, y1: f32, down: bool) {
    p.rounded(
        x0,
        y0,
        x1,
        y1,
        2.0,
        c(if down { FACE_DOWN } else { FACE }),
        Some(c(EDGE)),
    );
}

/// Text on one line, its middle at `cy`.
fn line(p: &mut Painter, text: &str, x: f32, cy: f32, size: f32, color: u32) {
    p.text(text, x, cy - size * 1.25 / 2.0, size, c(color));
}

/// Draw the control `id` of the sheet in `rect` and register the zone
/// that answers the mouse over it.
pub(super) fn control(p: &mut Painter, id: u32, rect: (f32, f32, f32, f32)) {
    let app = p.app;
    let Some(control) = app.wb.sheets[app.sheet].control(id) else {
        return;
    };
    let now = app.wb.control_state(app.sheet, control);
    let z = p.zoom();
    let (x0, y0, x1, y1) = rect;
    let (cy, size) = ((y0 + y1) / 2.0, TEXT * z);
    let held = pressed_part(app, id);
    let items = if control.kind.has_list() {
        app.wb.control_items(app.sheet, control)
    } else {
        Vec::new()
    };
    let top = list_top(app, id);
    let geometry = Geometry::of(&now, rect, z, items.len() as u32, top);
    // What is drawn stays in the box of the control.
    let outer = std::mem::take(&mut p.items);
    match control.kind {
        ControlKind::Button => {
            face(p, x0, y0, x1, y1, held == Some(Part::Face));
            let width = f32::from(p.shape(&now.text, size, c(INK), false).width);
            line(
                p,
                &now.text,
                ((x0 + x1 - width) / 2.0).max(x0 + 2.0 * z),
                cy,
                size,
                INK,
            );
        }
        ControlKind::CheckBox | ControlKind::OptionButton => {
            let side = 13.0 * z;
            let (bx, by) = (x0 + 2.0 * z, cy - side / 2.0);
            let down = held == Some(Part::Face);
            if control.kind == ControlKind::CheckBox {
                p.rect(bx, by, bx + side, by + side, c(EDGE));
                p.rect(
                    bx + 1.0,
                    by + 1.0,
                    bx + side - 1.0,
                    by + side - 1.0,
                    c(if down { FACE_DOWN } else { FIELD }),
                );
                match now.checked {
                    CheckState::Checked => {
                        let tick = [
                            (bx + side * 0.22, by + side * 0.52),
                            (bx + side * 0.42, by + side * 0.72),
                            (bx + side * 0.8, by + side * 0.26),
                        ];
                        p.polyline(&tick, 1.6 * z, c(INK));
                    }
                    CheckState::Mixed => p.rect(
                        bx + 3.0 * z,
                        by + 3.0 * z,
                        bx + side - 3.0 * z,
                        by + side - 3.0 * z,
                        c(EDGE),
                    ),
                    CheckState::Unchecked => {}
                }
            } else {
                let (ox, oy, r) = (bx + side / 2.0, cy, side / 2.0);
                p.disc(ox, oy, r, c(EDGE));
                p.disc(ox, oy, r - 1.0, c(if down { FACE_DOWN } else { FIELD }));
                if now.checked == CheckState::Checked {
                    p.disc(ox, oy, r * 0.45, c(INK));
                }
            }
            line(p, &now.text, bx + side + 5.0 * z, cy, size, INK);
        }
        ControlKind::GroupBox => {
            // The frame starts below the middle of the caption and leaves
            // room for it.
            let top_y = y0 + size * 0.6;
            let width = if now.text.is_empty() {
                0.0
            } else {
                f32::from(p.shape(&now.text, size, c(INK), false).width) + 6.0 * z
            };
            let (gap0, gap1) = (x0 + 6.0 * z, (x0 + 6.0 * z + width).min(x1));
            p.rect(x0, top_y, gap0, top_y + 1.0, c(EDGE));
            p.rect(gap1, top_y, x1, top_y + 1.0, c(EDGE));
            p.rect(x0, top_y, x0 + 1.0, y1, c(EDGE));
            p.rect(x1 - 1.0, top_y, x1, y1, c(EDGE));
            p.rect(x0, y1 - 1.0, x1, y1, c(EDGE));
            p.text(&now.text, gap0 + 3.0 * z, y0, size, c(INK));
        }
        ControlKind::Label => {
            p.text(&now.text, x0 + 1.0 * z, y0 + 1.0 * z, size, c(INK));
        }
        ControlKind::ComboBox => {
            p.rect(x0, y0, x1, y1, c(EDGE));
            p.rect(x0 + 1.0, y0 + 1.0, x1 - 1.0, y1 - 1.0, c(FIELD));
            let arrow = (y1 - y0).min(18.0 * z);
            if let Some(item) = now
                .selected
                .checked_sub(1)
                .and_then(|k| items.get(k as usize))
            {
                line(p, item, x0 + 4.0 * z, cy, size, INK);
            }
            face(p, x1 - arrow, y0 + 1.0, x1 - 1.0, y1 - 1.0, false);
            triangle(p, x1 - arrow / 2.0 - 0.5, cy, 3.5 * z, 1);
        }
        ControlKind::ListBox => {
            p.rect(x0, y0, x1, y1, c(EDGE));
            p.rect(x0 + 1.0, y0 + 1.0, x1 - 1.0, y1 - 1.0, c(FIELD));
            let row_h = LINE * z;
            for (k, item) in items
                .iter()
                .enumerate()
                .skip(top as usize)
                .take(geometry.lines_shown() as usize + 1)
            {
                let ry = y0 + 1.0 + (k as u32 - top) as f32 * row_h;
                let chosen = now.selected == k as u32 + 1;
                if chosen {
                    p.rect(
                        x0 + 1.0,
                        ry,
                        x1 - 1.0,
                        (ry + row_h).min(y1 - 1.0),
                        c(CHOSEN),
                    );
                }
                line(
                    p,
                    item,
                    x0 + 4.0 * z,
                    ry + row_h / 2.0,
                    size,
                    if chosen { FIELD } else { INK },
                );
            }
        }
        ControlKind::SpinButton => {
            let (mid, cx) = ((y0 + y1) / 2.0, (x0 + x1) / 2.0);
            let arrow = ((x1 - x0).min(mid - y0) * 0.22).max(2.0);
            face(p, x0, y0, x1, mid, held == Some(Part::Less));
            face(p, x0, mid, x1, y1, held == Some(Part::More));
            triangle(p, cx, (y0 + mid) / 2.0, arrow, 0);
            triangle(p, cx, (mid + y1) / 2.0, arrow, 1);
        }
        ControlKind::ScrollBar => {
            p.rect(x0, y0, x1, y1, c(EDGE));
            p.rect(x0 + 1.0, y0 + 1.0, x1 - 1.0, y1 - 1.0, c(0xE4E4E4));
            let (arrow, start, length) = (
                geometry.arrow(),
                geometry.thumb_start(),
                geometry.thumb_length(),
            );
            let mark = (geometry.bar().1 * 0.22).max(2.0);
            if geometry.horizontal {
                face(p, x0, y0, x0 + arrow, y1, false);
                face(p, x1 - arrow, y0, x1, y1, false);
                triangle(p, x0 + arrow / 2.0, cy, mark, 2);
                triangle(p, x1 - arrow / 2.0, cy, mark, 3);
                face(
                    p,
                    x0 + start,
                    y0,
                    x0 + start + length,
                    y1,
                    held == Some(Part::Thumb),
                );
            } else {
                let cx = (x0 + x1) / 2.0;
                face(p, x0, y0, x1, y0 + arrow, false);
                face(p, x0, y1 - arrow, x1, y1, false);
                triangle(p, cx, y0 + arrow / 2.0, mark, 0);
                triangle(p, cx, y1 - arrow / 2.0, mark, 1);
                face(
                    p,
                    x0,
                    y0 + start,
                    x1,
                    y0 + start + length,
                    held == Some(Part::Thumb),
                );
            }
        }
    }
    let items = std::mem::replace(&mut p.items, outer);
    if !items.is_empty() {
        let clip = p.layout.bounds_px(
            x0.max(p.clip.0),
            y0.max(p.clip.1),
            x1.min(p.clip.2),
            y1.min(p.clip.3),
        );
        p.push(Draw::Clipped { clip, items });
    }
    p.zone(rect, ControlZone { id, rect, zoom: z })
        .cursor(gpui::CursorStyle::Arrow);
}
