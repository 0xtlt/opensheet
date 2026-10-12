// TEMP-DEBUG: sample drawings for looking at the feature; removed before commit.
use super::tool;
use crate::app::ExcelApp;
use crate::features::Cx;
use opensheet_engine::drawing::{Dash, Frame, LineEnd, ShapeTool};

pub(super) fn seed(app: &mut ExcelApp, cx: &mut Cx) {
    let Ok(dir) = std::env::var("OPENSHEET_DRAWINGS_SEED") else {
        return;
    };
    let sheet = app.sheet;
    for (i, tool) in ShapeTool::ALL.into_iter().enumerate() {
        let (w, h) = tool.default_size();
        let (w, h) = (w as f64 * 0.8, if h == 0.0 { 30.0 } else { h as f64 * 0.8 });
        let frame = Frame::new(560.0 + 110.0 * (i % 6) as f64, 30.0 + 110.0 * (i / 6) as f64, w, h);
        if let Some(id) = app.wb.add_shape(sheet, tool, frame) {
            app.wb.update_drawing(sheet, id, |d| {
                if let Some(shape) = d.as_shape_mut() {
                    if !shape.preset.is_line() {
                        shape.text = if i == 16 { "Zone de texte avec un texte assez long pour passer à la ligne".into() } else { format!("Abc {i}") };
                    }
                    if i == 14 {
                        shape.outline.as_mut().unwrap().dash = Dash::Dash;
                        shape.outline.as_mut().unwrap().width = 2.25;
                    }
                    if i == 15 {
                        shape.outline.as_mut().unwrap().start = LineEnd::Oval;
                        shape.outline.as_mut().unwrap().end = LineEnd::Stealth;
                        shape.outline.as_mut().unwrap().width = 3.0;
                    }
                }
                if i == 8 {
                    d.rotation = 45.0;
                }
                if i == 3 {
                    d.rotation = 90.0;
                }
                if i == 9 {
                    d.flip_v = true;
                }
            });
        }
    }
    app.wb.commit("seed");
    let read = |name: &str| std::fs::read(format!("{dir}/{name}")).ok();
    if let Some(bytes) = read("test.png") {
        tool::insert_pictures(app, vec![bytes], Some((70.0, 240.0)), cx);
    }
    if let Some(bytes) = read("test.jpg") {
        tool::insert_pictures(app, vec![bytes], Some((90.0, 470.0)), cx);
        if let Some(id) = super::selected(app).map(|d| d.id) {
            app.wb.update_drawing(sheet, id, |d| {
                d.rotation = 30.0;
                d.size = (160.0, 100.0);
            });
        }
    }
    if let Some(bytes) = read("test.tiff") {
        tool::insert_pictures(app, vec![bytes], Some((330.0, 470.0)), cx);
        if let Some(id) = super::selected(app).map(|d| d.id) {
            app.wb.update_drawing(sheet, id, |d| {
                d.rotation = 90.0;
                d.flip_h = true;
                d.size = (160.0, 100.0);
            });
        }
    }
    app.wb.commit("seed pictures");
    app.deselect_object(cx);
}
