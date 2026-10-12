//! SmartArt on the sheet: a graphic is laid out by the engine as a list of
//! shapes (`SmartArt::pieces`), which are drawn here like any shape.

use super::geometry::{self, Placed};
use super::paint::{self, Brush};
use crate::grid::{Draw, FontResolver};
use crate::theme::ca;
use opensheet_engine::Locale;
use opensheet_engine::smartart::{Piece, SmartArt, placeholder};
use opensheet_engine::theme::Theme;

/// Where a piece of a graphic is drawn, the graphic being drawn in
/// `placed`: `natural` is the size the pieces were laid out for.
pub(super) fn placed_piece(piece: &Piece, placed: &Placed, natural: (f32, f32)) -> Placed {
    let (sx, sy) = (placed.width() / natural.0.max(1.0), placed.height() / natural.1.max(1.0));
    Placed {
        x0: placed.x0 + piece.x * sx,
        y0: placed.y0 + piece.y * sy,
        x1: placed.x0 + (piece.x + piece.width) * sx,
        y1: placed.y0 + (piece.y + piece.height) * sy,
        rotation: piece.rotation,
        flip_h: piece.flip_h,
        flip_v: false,
    }
}

/// The items that draw a graphic in a frame. A graphic is never turned:
/// its frame stands as its cells do. The shapes come first, then what is
/// written in them, so that shapes that overlap hide no text.
pub(super) fn draw(art: &SmartArt, theme: &Theme, placed: &Placed, natural: (f32, f32), brush: &Brush, fonts: Option<&FontResolver>, locale: &Locale, out: &mut Vec<Draw>) {
    let pieces = art.pieces(theme, natural.0, natural.1);
    for piece in &pieces {
        let at = placed_piece(piece, placed, natural);
        let size = (piece.width, piece.height);
        let mut shape = piece.shape.clone();
        shape.text.clear();
        // A fill that lets what is under it show is painted here.
        if let (Some(fill), true) = (shape.fill, piece.opacity < 1.0) {
            for line in geometry::flatten(&shape.path(size.0, size.1), &at).iter().filter(|line| line.closed) {
                brush.fill(&line.points, ca(fill.value(), piece.opacity.max(0.0)), out);
            }
            shape.fill = None;
        }
        paint::draw_shape(&shape, &at, size, brush, None, out);
    }
    let Some(fonts) = fonts else {
        return;
    };
    for piece in pieces.iter().filter(|piece| piece.node.is_some()) {
        let at = placed_piece(piece, placed, natural);
        let mut shape = piece.shape.clone();
        (shape.fill, shape.outline) = (None, None);
        // An entry nothing was typed in says so, on screen only.
        if shape.text.is_empty() {
            shape.text = placeholder(locale).to_string();
        }
        paint::draw_shape(&shape, &at, (piece.width, piece.height), brush, Some(fonts), out);
    }
}
