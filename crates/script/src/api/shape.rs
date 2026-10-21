//! Shapes and pictures floating over the cells: `Shape`, its fill, its
//! line, its text and, for a picture, its `Image`.
//!
//! Places and sizes are in points, as in Excel; the engine keeps pixels
//! at 100 %. A new shape is the one the Insert tab draws: same look,
//! same name, on top of the others.

use serde_json::Value as J;

use opensheet_engine::drawing::{
    Dash, Drawing, DrawingKind, Frame, ImageFormat, Outline, Placement, Shape, ShapePreset,
    ShapeTool, TextAlign, TextAnchor, ZOrder,
};
use opensheet_engine::protect::Action;
use opensheet_engine::style::{Color, Font, Underline};

use super::util::{base64_decode, base64_encode, color_arg, color_text};
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

const PX_PER_POINT: f64 = 96.0 / 72.0;

/// The geometric shapes OpenSheet draws, by their name in
/// `GeometricShapeType`.
const GEOMETRIC: [(&str, ShapeTool); 16] = [
    ("Rectangle", ShapeTool::Rectangle),
    ("RoundRectangle", ShapeTool::RoundedRectangle),
    ("Ellipse", ShapeTool::Ellipse),
    ("Triangle", ShapeTool::Triangle),
    ("RightTriangle", ShapeTool::RightTriangle),
    ("Diamond", ShapeTool::Diamond),
    ("Pentagon", ShapeTool::Pentagon),
    ("Hexagon", ShapeTool::Hexagon),
    ("RightArrow", ShapeTool::RightArrow),
    ("LeftArrow", ShapeTool::LeftArrow),
    ("UpArrow", ShapeTool::UpArrow),
    ("DownArrow", ShapeTool::DownArrow),
    ("Star5", ShapeTool::Star5),
    ("WedgeRectCallout", ShapeTool::Callout),
    ("Chevron", ShapeTool::Chevron),
    ("Trapezoid", ShapeTool::Trapezoid),
];

/// The dashes of a line: the name of each in `ShapeLineDashStyle`, and
/// the names that are read as it.
const DASHES: [(Dash, &str, &[&str]); 11] = [
    (Dash::Solid, "Solid", &[]),
    (Dash::SysDot, "RoundDot", &[]),
    (Dash::SysDash, "SquareDot", &["SystemDash"]),
    (Dash::Dash, "Dash", &[]),
    (Dash::DashDot, "DashDot", &[]),
    (Dash::LongDash, "LongDash", &[]),
    (Dash::LongDashDot, "LongDashDot", &[]),
    (Dash::LongDashDotDot, "LongDashDotDot", &[]),
    (Dash::Dot, "SystemDot", &[]),
    (Dash::SysDashDot, "SystemDashDot", &[]),
    (Dash::SysDashDotDot, "DashDotDot", &[]),
];

fn shape_object(h: &Host, sheet: usize, id: u32) -> J {
    object("Shape", vec![h.sheet_id(sheet).into(), id.into()])
}

fn part_object(call: &Call, class: &str) -> J {
    object(class, call.this.to_vec())
}

/// The sheet and the drawing an object of these classes stands for.
fn drawing<'h>(h: &'h Host, call: &Call) -> Res<(usize, &'h Drawing)> {
    let sheet = h.sheet(call, call.this_int(0))?;
    let id = u32::try_from(call.this_int(1)).map_err(|_| call.gone())?;
    Ok((sheet, h.wb.drawing(sheet, id).ok_or_else(|| call.gone())?))
}

/// The geometric shape, line or text box behind an object.
fn shape<'h>(h: &'h Host, call: &Call) -> Res<&'h Shape> {
    drawing(h, call)?
        .1
        .as_shape()
        .ok_or_else(|| call.fail("This is not a geometric shape, a line or a text box."))
}

/// Change the drawing a call is made on. A protected sheet refuses
/// unless it lets objects be edited, or the object is unlocked.
fn edit(h: &mut Host, call: &Call, change: impl FnOnce(&mut Drawing) -> Res<()>) -> Res<Ret> {
    let (sheet, held) = drawing(h, call)?;
    let (id, locked) = (held.id, held.locked);
    h.permit(call, &Action::EditObject { sheet, locked })?;
    let mut done = Ok(());
    h.wb.update_drawing(sheet, id, |drawing| {
        let mut changed = drawing.clone();
        done = change(&mut changed);
        if done.is_ok() {
            *drawing = changed;
        }
    });
    done.map(|_| Ret::Nothing)
}

fn edit_shape(h: &mut Host, call: &Call, change: impl FnOnce(&mut Shape) -> Res<()>) -> Res<Ret> {
    shape(h, call)?;
    edit(h, call, |drawing| {
        drawing.as_shape_mut().map_or(Ok(()), change)
    })
}

/// The fonts of the text of a shape: its own, and those of its runs.
fn edit_font(h: &mut Host, call: &Call, change: impl Fn(&mut Font)) -> Res<Ret> {
    edit_shape(h, call, |shape| {
        change(&mut shape.font);
        shape.runs.iter_mut().for_each(|run| change(&mut run.font));
        Ok(())
    })
}

fn frame(h: &Host, call: &Call) -> Res<Frame> {
    let (sheet, held) = drawing(h, call)?;
    h.wb.drawing_frame(sheet, held.id)
        .ok_or_else(|| call.gone())
}

/// Move or resize, in points: `change` gets the frame in points.
fn set_frame(h: &mut Host, call: &Call, change: impl FnOnce(&mut Frame)) -> Res<Ret> {
    let (sheet, held) = drawing(h, call)?;
    let (id, locked) = (held.id, held.locked);
    let px = frame(h, call)?;
    let mut points = Frame::new(
        px.x / PX_PER_POINT,
        px.y / PX_PER_POINT,
        px.width / PX_PER_POINT,
        px.height / PX_PER_POINT,
    );
    change(&mut points);
    let sane = |n: f64| n.is_finite() && (0.0..=1_000_000.0).contains(&n);
    if ![points.x, points.y, points.width, points.height]
        .into_iter()
        .all(sane)
    {
        return Err(call.invalid());
    }
    h.permit(call, &Action::EditObject { sheet, locked })?;
    let new = Frame::new(
        points.x * PX_PER_POINT,
        points.y * PX_PER_POINT,
        points.width * PX_PER_POINT,
        points.height * PX_PER_POINT,
    );
    h.wb.set_drawing_frame(sheet, id, new);
    Ok(Ret::Nothing)
}

fn turned(angle: f64) -> f32 {
    angle.rem_euclid(360.0) as f32
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::Bmp => "BMP",
    }
}

// ---- adding and finding ----------------------------------------------------------------

/// Put a new shape on a sheet, where a click of its tool would.
fn insert(h: &mut Host, call: &Call, sheet: usize, tool: ShapeTool, frame: Frame) -> Res<u32> {
    h.permit(
        call,
        &Action::EditObject {
            sheet,
            locked: true,
        },
    )?;
    h.wb.add_shape(sheet, tool, frame)
        .ok_or_else(|| call.gone())
}

fn default_frame(tool: ShapeTool) -> Frame {
    let (width, height) = tool.default_size();
    Frame::new(0.0, 0.0, width as f64, height as f64)
}

pub(super) fn add_geometric(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    // Reading a member of the enum that OpenSheet does not draw already
    // failed with its name.
    let name = enum_value(call, 0, "GeometricShapeType")?.ok_or_else(|| call.invalid())?;
    let tool = GEOMETRIC
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, tool)| *tool)
        .ok_or_else(|| call.invalid())?;
    let id = insert(h, call, sheet, tool, default_frame(tool))?;
    Ok(Ret::Object(shape_object(h, sheet, id)))
}

pub(super) fn add_text_box(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let text = call.opt_str(0)?.unwrap_or_default();
    let id = insert(
        h,
        call,
        sheet,
        ShapeTool::TextBox,
        default_frame(ShapeTool::TextBox),
    )?;
    h.wb.update_drawing(sheet, id, |drawing| {
        if let Some(shape) = drawing.as_shape_mut() {
            shape.text = text.to_string();
        }
    });
    Ok(Ret::Object(shape_object(h, sheet, id)))
}

pub(super) fn add_line(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let point = |index: usize| -> Res<f64> {
        let n = call.num(index)?;
        if n.is_finite() && (0.0..=1_000_000.0).contains(&n) {
            Ok(n * PX_PER_POINT)
        } else {
            Err(call.invalid())
        }
    };
    let (x0, y0, x1, y1) = (point(0)?, point(1)?, point(2)?, point(3)?);
    match enum_value(call, 4, "ConnectorType")? {
        None | Some("Straight") => {}
        Some(other) => {
            return Err(call.unsupported(format!(
                "ExcelScript.ConnectorType.{}{}",
                other[..1].to_lowercase(),
                &other[1..]
            )));
        }
    }
    let frame = Frame::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
    let id = insert(h, call, sheet, ShapeTool::Line, frame)?;
    // A line runs from the top left of its frame to the bottom right
    // unless it is flipped.
    h.wb.update_drawing(sheet, id, |drawing| {
        drawing.flip_h = x1 < x0;
        drawing.flip_v = y1 < y0;
    });
    Ok(Ret::Object(shape_object(h, sheet, id)))
}

pub(super) fn add_image(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let text = call.str(0)?;
    // A data URL is accepted too: what follows its comma is the image.
    let encoded = text.split_once("base64,").map_or(text, |(_, rest)| rest);
    let data =
        base64_decode(encoded).ok_or_else(|| call.fail("The image is not valid base64 text."))?;
    h.permit(
        call,
        &Action::EditObject {
            sheet,
            locked: true,
        },
    )?;
    let id =
        h.wb.add_picture(sheet, data, (0, 0))
            .ok_or_else(|| call.fail("The image is not a PNG, JPEG, GIF or BMP picture."))?;
    Ok(Ret::Object(shape_object(h, sheet, id)))
}

/// The shapes and pictures of a sheet, the bottom-most first.
pub(super) fn all(h: &Host, sheet: usize) -> Ret {
    let mut held: Vec<&Drawing> = h.wb.sheets[sheet].drawings.iter().collect();
    held.sort_by_key(|drawing| (drawing.z == 0, drawing.z));
    Ret::Object(J::from(
        held.into_iter()
            .map(|drawing| shape_object(h, sheet, drawing.id))
            .collect::<Vec<J>>(),
    ))
}

/// `getShape(key)`: by name or by ID.
pub(super) fn find(h: &Host, call: &Call, sheet: usize) -> Res<Ret> {
    let key = call.str(0)?;
    let drawings = &h.wb.sheets[sheet].drawings;
    let found = drawings
        .iter()
        .find(|drawing| drawing.name.eq_ignore_ascii_case(key))
        .or_else(|| {
            drawings
                .iter()
                .find(|drawing| drawing.id.to_string() == key)
        });
    Ok(Ret::maybe(
        found.map(|drawing| shape_object(h, sheet, drawing.id)),
    ))
}

// ---- the classes -------------------------------------------------------------------------

pub(super) const SHAPE: ApiClass = class!("Shape", "A shape, a line, a text box or a picture floating over the cells of a worksheet.", {
    getId "(): string" "The identifier of the shape on its worksheet." => |h, c| Ok(drawing(h, c)?.1.id.to_string().into());
    getName "(): string" "The name of the shape, as the Selection pane lists it." => |h, c| Ok(drawing(h, c)?.1.name.clone().into());
    setName "(name: string): void" "Renames the shape." => |h, c| {
        let name = c.str(0)?.trim();
        if name.is_empty() {
            return Err(c.invalid());
        }
        edit(h, c, |drawing| {
            drawing.name = name.to_string();
            Ok(())
        })
    };
    getType "(): ExcelScript.ShapeType" "What it is: GeometricShape, Line, Image, Group, or Unsupported for SmartArt." => |h, c| {
        Ok(match &drawing(h, c)?.1.kind {
            DrawingKind::Picture(_) => "Image",
            DrawingKind::Shape(shape) if shape.preset.is_line() => "Line",
            DrawingKind::Shape(_) => "GeometricShape",
            DrawingKind::Group(_) => "Group",
            DrawingKind::SmartArt(_) => "Unsupported",
        }.into())
    };
    getGeometricShapeType "(): ExcelScript.GeometricShapeType" "The geometric shape: Rectangle, Ellipse, RightArrow... Fails for a line or a picture." => |h, c| {
        let preset = shape(h, c)?.preset;
        let name = GEOMETRIC.iter().find(|(_, tool)| tool.preset() == preset).map(|(name, _)| *name);
        name.map(Into::into).ok_or_else(|| c.fail("This is not a geometric shape."))
    };
    setGeometricShapeType "(geometricShapeType: ExcelScript.GeometricShapeType): void" "Changes the geometric shape, keeping its place, its look and its text." => |h, c| {
        let name = enum_value(c, 0, "GeometricShapeType")?.ok_or_else(|| c.invalid())?;
        let preset = GEOMETRIC.iter().find(|(n, _)| *n == name).map(|(_, tool)| tool.preset()).ok_or_else(|| c.invalid())?;
        if shape(h, c)?.preset == ShapePreset::Line {
            return Err(c.fail("This is not a geometric shape."));
        }
        edit_shape(h, c, |shape| {
            shape.preset = preset;
            shape.adjust = None;
            Ok(())
        })
    };
    getLeft "(): number" "The distance from the left edge of the worksheet, in points." => |h, c| Ok((frame(h, c)?.x / PX_PER_POINT).into());
    setLeft "(left: number): void" "Moves the shape: its distance from the left edge of the worksheet, in points." => |h, c| {
        let left = c.num(0)?;
        set_frame(h, c, |frame| frame.x = left)
    };
    getTop "(): number" "The distance from the top edge of the worksheet, in points." => |h, c| Ok((frame(h, c)?.y / PX_PER_POINT).into());
    setTop "(top: number): void" "Moves the shape: its distance from the top edge of the worksheet, in points." => |h, c| {
        let top = c.num(0)?;
        set_frame(h, c, |frame| frame.y = top)
    };
    getWidth "(): number" "The width of the shape, in points." => |h, c| Ok((frame(h, c)?.width / PX_PER_POINT).into());
    setWidth "(width: number): void" "Sets the width of the shape, in points." => |h, c| {
        let width = c.num(0)?;
        set_frame(h, c, |frame| frame.width = width)
    };
    getHeight "(): number" "The height of the shape, in points." => |h, c| Ok((frame(h, c)?.height / PX_PER_POINT).into());
    setHeight "(height: number): void" "Sets the height of the shape, in points." => |h, c| {
        let height = c.num(0)?;
        set_frame(h, c, |frame| frame.height = height)
    };
    incrementLeft "(increment: number): void" "Moves the shape to the right by so many points (to the left when negative)." => |h, c| {
        let by = c.num(0)?;
        set_frame(h, c, |frame| frame.x += by)
    };
    incrementTop "(increment: number): void" "Moves the shape down by so many points (up when negative)." => |h, c| {
        let by = c.num(0)?;
        set_frame(h, c, |frame| frame.y += by)
    };
    getRotation "(): number" "The rotation of the shape, in degrees clockwise." => |h, c| Ok((drawing(h, c)?.1.rotation as f64).into());
    setRotation "(rotation: number): void" "Turns the shape: degrees clockwise." => |h, c| {
        let angle = c.num(0)?;
        if !angle.is_finite() {
            return Err(c.invalid());
        }
        edit(h, c, |drawing| {
            drawing.rotation = turned(angle);
            Ok(())
        })
    };
    incrementRotation "(increment: number): void" "Turns the shape further by so many degrees." => |h, c| {
        let by = c.num(0)?;
        if !by.is_finite() {
            return Err(c.invalid());
        }
        edit(h, c, |drawing| {
            drawing.rotation = turned(drawing.rotation as f64 + by);
            Ok(())
        })
    };
    getVisible "(): boolean" "Whether the shape shows." => |h, c| Ok((!drawing(h, c)?.1.hidden).into());
    setVisible "(visible: boolean): void" "Shows or hides the shape, as the eye of the Selection pane." => |h, c| {
        let visible = c.bool(0)?;
        edit(h, c, |drawing| {
            drawing.hidden = !visible;
            Ok(())
        })
    };
    getAltTextDescription "(): string" "The alternative text of the shape." => |h, c| Ok(drawing(h, c)?.1.alt_text.clone().into());
    setAltTextDescription "(altTextDescription: string): void" "Sets the alternative text of the shape." => |h, c| {
        let text = c.str(0)?;
        edit(h, c, |drawing| {
            drawing.alt_text = text.to_string();
            Ok(())
        })
    };
    getLockAspectRatio "(): boolean" "Whether the shape keeps its proportions when it is resized by hand." => |h, c| Ok(drawing(h, c)?.1.lock_aspect.into());
    setLockAspectRatio "(lockAspectRatio: boolean): void" "Lock aspect ratio: on or off." => |h, c| {
        let lock = c.bool(0)?;
        edit(h, c, |drawing| {
            drawing.lock_aspect = lock;
            Ok(())
        })
    };
    getPlacement "(): ExcelScript.Placement" "How the shape follows the cells under it: TwoCell (moves and sizes with them), OneCell (moves) or Absolute." => |h, c| {
        Ok(match drawing(h, c)?.1.placement {
            Placement::MoveAndSize => "TwoCell",
            Placement::MoveOnly => "OneCell",
            Placement::Fixed => "Absolute",
        }.into())
    };
    setPlacement "(placement: ExcelScript.Placement): void" "Sets how the shape follows the cells under it." => |h, c| {
        let placement = match enum_value(c, 0, "Placement")?.ok_or_else(|| c.invalid())? {
            "TwoCell" => Placement::MoveAndSize,
            "OneCell" => Placement::MoveOnly,
            _ => Placement::Fixed,
        };
        edit(h, c, |drawing| {
            drawing.placement = placement;
            Ok(())
        })
    };
    getZOrderPosition "(): number" "The place of the shape among the charts, pictures and shapes of the worksheet: 0 is the bottom-most." => |h, c| {
        let (sheet, held) = drawing(h, c)?;
        let stack = h.wb.object_stack(sheet);
        let place = stack.iter().position(|object| *object == opensheet_engine::drawing::ObjectRef::Drawing(held.id));
        Ok((place.unwrap_or(0) as u64).into())
    };
    setZOrder "(position: ExcelScript.ShapeZOrder): void" "Bring to Front, Bring Forward, Send Backward or Send to Back." => |h, c| {
        let order = match enum_value(c, 0, "ShapeZOrder")?.ok_or_else(|| c.invalid())? {
            "BringToFront" => ZOrder::BringToFront,
            "BringForward" => ZOrder::BringForward,
            "SendBackward" => ZOrder::SendBackward,
            _ => ZOrder::SendToBack,
        };
        let (sheet, held) = drawing(h, c)?;
        let (id, locked) = (held.id, held.locked);
        h.permit(c, &Action::EditObject { sheet, locked })?;
        h.wb.reorder_drawing(sheet, id, order);
        Ok(Ret::Nothing)
    };
    getFill "(): ShapeFill" "The fill of the shape." => |h, c| {
        drawing(h, c)?;
        Ok(Ret::Object(part_object(c, "ShapeFill")))
    };
    getLineFormat "(): ShapeLineFormat" "The line of the shape: the line itself, or the outline of a geometric shape." => |h, c| {
        drawing(h, c)?;
        Ok(Ret::Object(part_object(c, "ShapeLineFormat")))
    };
    getTextFrame "(): TextFrame" "The text of the shape and how it is placed in it." => |h, c| {
        drawing(h, c)?;
        Ok(Ret::Object(part_object(c, "TextFrame")))
    };
    getImage "(): Image" "The picture, for a shape that is one. Fails for the others." => |h, c| {
        match drawing(h, c)?.1.as_picture() {
            Some(_) => Ok(Ret::Object(part_object(c, "Image"))),
            None => Err(c.fail("The shape is not a picture.")),
        }
    };
    getImageAsBase64 "(format: ExcelScript.PictureFormat): string" "The file of a picture as base64 text, in the format it has (pictures are not converted, and other shapes are not drawn as pictures)." => |h, c| {
        let wanted = enum_value(c, 0, "PictureFormat")?.ok_or_else(|| c.invalid())?;
        let Some(picture) = drawing(h, c)?.1.as_picture() else {
            return Err(c.unsupported("Shape.getImageAsBase64 for a shape that is not a picture"));
        };
        if format_name(picture.format) != wanted {
            return Err(c.unsupported(format!("Shape.getImageAsBase64: converting a {} picture to {wanted}", format_name(picture.format))));
        }
        Ok(base64_encode(&picture.data).into())
    };
    delete "(): void" "Deletes the shape." => |h, c| {
        let (sheet, held) = drawing(h, c)?;
        let (id, locked) = (held.id, held.locked);
        h.permit(c, &Action::EditObject { sheet, locked })?;
        h.wb.remove_drawing(sheet, id);
        Ok(Ret::Nothing)
    };
});

pub(super) const SHAPE_FILL: ApiClass = class!("ShapeFill", "The fill of a shape.", {
    getType "(): ExcelScript.ShapeFillType" "Solid, or NoFill (lines and pictures have none)." => |h, c| {
        Ok(if drawing(h, c)?.1.as_shape().is_some_and(|shape| shape.fill.is_some()) { "Solid" } else { "NoFill" }.into())
    };
    getForegroundColor "(): string" "The fill colour as #RRGGBB; empty text without a fill." => |h, c| {
        Ok(drawing(h, c)?.1.as_shape().and_then(|shape| shape.fill).map(color_text).unwrap_or_default().into())
    };
    setForegroundColor "(foregroundColor: string): void" "Fills the shape with a colour (#RRGGBB or a colour name)." => |h, c| {
        let color = color_arg(c, 0)?.ok_or_else(|| c.invalid())?;
        edit_shape(h, c, |shape| {
            shape.fill = Some(color);
            Ok(())
        })
    };
    setSolidColor "(color: string): void" "Fills the shape with one colour (#RRGGBB or a colour name)." => |h, c| {
        let color = color_arg(c, 0)?.ok_or_else(|| c.invalid())?;
        edit_shape(h, c, |shape| {
            shape.fill = Some(color);
            Ok(())
        })
    };
    clear "(): void" "No Fill." => |h, c| {
        edit_shape(h, c, |shape| {
            shape.fill = None;
            Ok(())
        })
    };
});

/// The outline of a shape, drawn in black when it had none.
fn outline(shape: &mut Shape) -> &mut Outline {
    shape
        .outline
        .get_or_insert(Outline::solid(Color::BLACK, 1.0))
}

pub(super) const SHAPE_LINE: ApiClass = class!("ShapeLineFormat", "The line of a shape: a line itself, or the outline of a geometric shape or a text box.", {
    getVisible "(): boolean" "Whether the shape has a line." => |h, c| Ok(drawing(h, c)?.1.as_shape().is_some_and(|shape| shape.outline.is_some()).into());
    setVisible "(visible: boolean): void" "Draws the line (black, one point, when the shape had none), or No Outline." => |h, c| {
        let visible = c.bool(0)?;
        edit_shape(h, c, |shape| {
            if visible {
                outline(shape);
            } else {
                shape.outline = None;
            }
            Ok(())
        })
    };
    getColor "(): string" "The colour of the line as #RRGGBB; empty text without a line." => |h, c| {
        Ok(drawing(h, c)?.1.as_shape().and_then(|shape| shape.outline).map(|line| color_text(line.color)).unwrap_or_default().into())
    };
    setColor "(color: string): void" "Colours the line (#RRGGBB or a colour name)." => |h, c| {
        let color = color_arg(c, 0)?.ok_or_else(|| c.invalid())?;
        edit_shape(h, c, |shape| {
            outline(shape).color = color;
            Ok(())
        })
    };
    getWeight "(): number" "The weight of the line, in points; 0 without a line." => |h, c| {
        Ok((drawing(h, c)?.1.as_shape().and_then(|shape| shape.outline).map_or(0.0, |line| line.width) as f64).into())
    };
    setWeight "(weight: number): void" "Sets the weight of the line, in points." => |h, c| {
        let weight = c.num(0)?;
        if !(weight > 0.0 && weight <= 1584.0) {
            return Err(c.invalid());
        }
        edit_shape(h, c, |shape| {
            outline(shape).width = weight as f32;
            Ok(())
        })
    };
    getDashStyle "(): ExcelScript.ShapeLineDashStyle" "The dashes of the line: Solid, RoundDot, SquareDot, Dash, DashDot, LongDash..." => |h, c| {
        let dash = shape(h, c)?.outline.map(|line| line.dash).unwrap_or_default();
        Ok(DASHES.iter().find(|(d, _, _)| *d == dash).map_or("Solid", |(_, name, _)| name).into())
    };
    setDashStyle "(dashStyle: ExcelScript.ShapeLineDashStyle): void" "Sets the dashes of the line." => |h, c| {
        let name = enum_value(c, 0, "ShapeLineDashStyle")?.ok_or_else(|| c.invalid())?;
        let dash = DASHES.iter().find(|(_, own, others)| *own == name || others.contains(&name)).map(|(dash, _, _)| *dash).ok_or_else(|| c.invalid())?;
        edit_shape(h, c, |shape| {
            outline(shape).dash = dash;
            Ok(())
        })
    };
});

pub(super) const TEXT_FRAME: ApiClass = class!("TextFrame", "The text of a shape and its place in the shape.", {
    getHasText "(): boolean" "Whether the shape holds text." => |h, c| Ok(drawing(h, c)?.1.as_shape().is_some_and(|shape| !shape.text.is_empty()).into());
    getTextRange "(): TextRange" "The text of the shape." => |h, c| {
        shape(h, c)?;
        Ok(Ret::Object(part_object(c, "TextRange")))
    };
    deleteText "(): void" "Removes the text of the shape." => |h, c| {
        edit_shape(h, c, |shape| {
            shape.text.clear();
            shape.runs.clear();
            Ok(())
        })
    };
    getHorizontalAlignment "(): ExcelScript.ShapeTextHorizontalAlignment" "How lines of text are aligned: Left, Center, Right or Justify." => |h, c| {
        Ok(match shape(h, c)?.align {
            TextAlign::Left => "Left",
            TextAlign::Center => "Center",
            TextAlign::Right => "Right",
            TextAlign::Justify => "Justify",
        }.into())
    };
    setHorizontalAlignment "(horizontalAlignment: ExcelScript.ShapeTextHorizontalAlignment): void" "Aligns the lines of text: Left, Center, Right or Justify." => |h, c| {
        let align = match enum_value(c, 0, "ShapeTextHorizontalAlignment")?.ok_or_else(|| c.invalid())? {
            "Left" => TextAlign::Left,
            "Center" => TextAlign::Center,
            "Right" => TextAlign::Right,
            "Justify" => TextAlign::Justify,
            other => return Err(c.unsupported(format!("ExcelScript.ShapeTextHorizontalAlignment.{}{}", other[..1].to_lowercase(), &other[1..]))),
        };
        edit_shape(h, c, |shape| {
            shape.align = align;
            Ok(())
        })
    };
    getVerticalAlignment "(): ExcelScript.ShapeTextVerticalAlignment" "Where the text stands in the shape: Top, Middle or Bottom." => |h, c| {
        Ok(match shape(h, c)?.anchor {
            TextAnchor::Top => "Top",
            TextAnchor::Middle => "Middle",
            TextAnchor::Bottom => "Bottom",
        }.into())
    };
    setVerticalAlignment "(verticalAlignment: ExcelScript.ShapeTextVerticalAlignment): void" "Places the text at the top, the middle or the bottom of the shape." => |h, c| {
        let anchor = match enum_value(c, 0, "ShapeTextVerticalAlignment")?.ok_or_else(|| c.invalid())? {
            "Top" => TextAnchor::Top,
            "Middle" => TextAnchor::Middle,
            "Bottom" => TextAnchor::Bottom,
            other => return Err(c.unsupported(format!("ExcelScript.ShapeTextVerticalAlignment.{}{}", other[..1].to_lowercase(), &other[1..]))),
        };
        edit_shape(h, c, |shape| {
            shape.anchor = anchor;
            Ok(())
        })
    };
});

pub(super) const TEXT_RANGE: ApiClass = class!("TextRange", "The text of a shape.", {
    getText "(): string" "The text." => |h, c| Ok(shape(h, c)?.text.clone().into());
    setText "(text: string): void" "Replaces the text; it takes the font of the shape." => |h, c| {
        let text = c.str(0)?;
        edit_shape(h, c, |shape| {
            shape.text = text.to_string();
            shape.runs.clear();
            Ok(())
        })
    };
    getFont "(): ShapeFont" "The font of the text." => |h, c| {
        shape(h, c)?;
        Ok(Ret::Object(part_object(c, "ShapeFont")))
    };
});

pub(super) const SHAPE_FONT: ApiClass = class!("ShapeFont", "The font of the text of a shape.", {
    getName "(): string" "The name of the font." => |h, c| Ok(shape(h, c)?.font.name.to_string().into());
    setName "(name: string): void" "Changes the font." => |h, c| {
        let name = c.str(0)?.trim();
        if name.is_empty() {
            return Err(c.invalid());
        }
        edit_font(h, c, |font| font.set_name(name))
    };
    getSize "(): number" "The size of the text, in points." => |h, c| Ok((shape(h, c)?.font.size_pt() as f64).into());
    setSize "(size: number): void" "Sets the size of the text, in points (1 to 409)." => |h, c| {
        let size = c.num(0)?;
        if !(1.0..=409.0).contains(&size) {
            return Err(c.invalid());
        }
        edit_font(h, c, |font| font.set_size_pt(size as f32))
    };
    getBold "(): boolean" "Whether the text is bold." => |h, c| Ok(shape(h, c)?.font.bold.into());
    setBold "(bold: boolean): void" "Bold or not." => |h, c| {
        let on = c.bool(0)?;
        edit_font(h, c, |font| font.bold = on)
    };
    getItalic "(): boolean" "Whether the text is italic." => |h, c| Ok(shape(h, c)?.font.italic.into());
    setItalic "(italic: boolean): void" "Italic or not." => |h, c| {
        let on = c.bool(0)?;
        edit_font(h, c, |font| font.italic = on)
    };
    getUnderline "(): ExcelScript.ShapeFontUnderlineStyle" "The underline: None, Single or Double." => |h, c| {
        Ok(match shape(h, c)?.font.underline {
            Underline::None => "None",
            Underline::Single => "Single",
            Underline::Double => "Double",
        }.into())
    };
    setUnderline "(underline: ExcelScript.ShapeFontUnderlineStyle): void" "Underlines the text: None, Single or Double." => |h, c| {
        let underline = match enum_value(c, 0, "ShapeFontUnderlineStyle")?.ok_or_else(|| c.invalid())? {
            "None" => Underline::None,
            "Single" => Underline::Single,
            "Double" => Underline::Double,
            other => return Err(c.unsupported(format!("ExcelScript.ShapeFontUnderlineStyle.{}{}", other[..1].to_lowercase(), &other[1..]))),
        };
        edit_font(h, c, |font| font.underline = underline)
    };
    getColor "(): string" "The colour of the text as #RRGGBB; empty text for the automatic colour." => |h, c| {
        Ok(shape(h, c)?.font.color.map(color_text).unwrap_or_default().into())
    };
    setColor "(color: string): void" "Colours the text (#RRGGBB or a colour name); empty text gives the automatic colour." => |h, c| {
        let color = color_arg(c, 0)?;
        edit_font(h, c, |font| font.color = color)
    };
});

pub(super) const IMAGE: ApiClass = class!("Image", "A picture: a shape that shows an image file.", {
    getId "(): string" "The identifier of the picture: that of its shape." => |h, c| Ok(drawing(h, c)?.1.id.to_string().into());
    getFormat "(): ExcelScript.PictureFormat" "The format of the image file: PNG, JPEG, GIF or BMP; SVG for a graphic kept as one." => |h, c| {
        let picture = drawing(h, c)?.1.as_picture().ok_or_else(|| c.gone())?;
        Ok(if picture.svg.is_some() { "SVG" } else { format_name(picture.format) }.into())
    };
    getShape "(): Shape" "The shape that holds the picture." => |h, c| {
        drawing(h, c)?;
        Ok(Ret::Object(part_object(c, "Shape")))
    };
});
