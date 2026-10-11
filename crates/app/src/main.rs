//! OpenSheet — an open-source spreadsheet built with Rust and GPUI.

mod app;
mod demo;
mod file;
mod grid;
mod i18n;
mod i18n_tables;
mod icons;
mod ribbon;
mod text_input;
mod theme;
mod view;

use app::*;
use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, point,
    prelude::*, px, size,
};
use i18n::t;

fn bind_keys(cx: &mut App) {
    text_input::bind_keys(cx);
    let g = Some("Grid");
    let w = Some("Workbook");
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, g),
        KeyBinding::new("down", MoveDown, g),
        KeyBinding::new("left", MoveLeft, g),
        KeyBinding::new("right", MoveRight, g),
        KeyBinding::new("shift-up", ExtendUp, g),
        KeyBinding::new("shift-down", ExtendDown, g),
        KeyBinding::new("shift-left", ExtendLeft, g),
        KeyBinding::new("shift-right", ExtendRight, g),
        KeyBinding::new("cmd-up", JumpUp, g),
        KeyBinding::new("cmd-down", JumpDown, g),
        KeyBinding::new("cmd-left", JumpLeft, g),
        KeyBinding::new("cmd-right", JumpRight, g),
        KeyBinding::new("ctrl-up", JumpUp, g),
        KeyBinding::new("ctrl-down", JumpDown, g),
        KeyBinding::new("ctrl-left", JumpLeft, g),
        KeyBinding::new("ctrl-right", JumpRight, g),
        KeyBinding::new("cmd-shift-up", ExtendJumpUp, g),
        KeyBinding::new("cmd-shift-down", ExtendJumpDown, g),
        KeyBinding::new("cmd-shift-left", ExtendJumpLeft, g),
        KeyBinding::new("cmd-shift-right", ExtendJumpRight, g),
        KeyBinding::new("ctrl-shift-up", ExtendJumpUp, g),
        KeyBinding::new("ctrl-shift-down", ExtendJumpDown, g),
        KeyBinding::new("ctrl-shift-left", ExtendJumpLeft, g),
        KeyBinding::new("ctrl-shift-right", ExtendJumpRight, g),
        KeyBinding::new("tab", NextCell, g),
        KeyBinding::new("shift-tab", PrevCell, g),
        KeyBinding::new("enter", EnterDown, g),
        KeyBinding::new("shift-enter", EnterUp, g),
        KeyBinding::new("f2", EditCell, g),
        KeyBinding::new("ctrl-u", EditCell, g),
        KeyBinding::new("backspace", ClearContents, g),
        KeyBinding::new("delete", ClearContents, g),
        KeyBinding::new("cmd-c", Copy, g),
        KeyBinding::new("cmd-x", Cut, g),
        KeyBinding::new("cmd-v", Paste, g),
        KeyBinding::new("cmd-shift-v", PasteValues, g),
        KeyBinding::new("cmd-a", SelectAll, g),
        KeyBinding::new("cmd-d", FillDown, g),
        KeyBinding::new("cmd-r", FillRight, g),
        KeyBinding::new("pageup", PageUp, g),
        KeyBinding::new("pagedown", PageDown, g),
        KeyBinding::new("home", GoRowStart, g),
        KeyBinding::new("cmd-home", GoStart, g),
        KeyBinding::new("ctrl-home", GoStart, g),
        KeyBinding::new("cmd-end", GoEnd, g),
        KeyBinding::new("ctrl-end", GoEnd, g),
        KeyBinding::new("ctrl-space", SelectColumn, g),
        KeyBinding::new("shift-space", SelectRow, g),
        KeyBinding::new("ctrl-;", InsertDate, g),
        KeyBinding::new("cmd-shift-t", AutoSum, g),
        KeyBinding::new("ctrl-shift-=", InsertRows, g),
        KeyBinding::new("ctrl--", DeleteRows, g),
        KeyBinding::new("escape", Cancel, g),
        // Window-wide
        KeyBinding::new("cmd-z", Undo, w),
        KeyBinding::new("cmd-shift-z", Redo, w),
        KeyBinding::new("cmd-y", Redo, w),
        KeyBinding::new("cmd-b", Bold, w),
        KeyBinding::new("cmd-i", Italic, w),
        KeyBinding::new("cmd-u", UnderlineText, w),
        KeyBinding::new("cmd-s", Save, w),
        KeyBinding::new("cmd-shift-s", SaveAs, w),
        KeyBinding::new("cmd-o", Open, w),
        KeyBinding::new("cmd-n", NewWorkbook, w),
        KeyBinding::new("cmd-f", Find, w),
        KeyBinding::new("cmd-=", ZoomIn, w),
        KeyBinding::new("cmd--", ZoomOut, w),
        KeyBinding::new("cmd-0", ZoomReset, w),
        KeyBinding::new("ctrl-pagedown", NextSheet, w),
        KeyBinding::new("ctrl-pageup", PrevSheet, w),
        KeyBinding::new("alt-right", NextSheet, g),
        KeyBinding::new("alt-left", PrevSheet, g),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

pub(crate) fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "OpenSheet".into(),
            items: vec![MenuItem::action(t("Quit OpenSheet", "Quitter OpenSheet"), Quit)],
        },
        Menu {
            name: t("File", "Fichier").into(),
            items: vec![
                MenuItem::action(t("New", "Nouveau"), NewWorkbook),
                MenuItem::action(t("Open…", "Ouvrir…"), Open),
                MenuItem::separator(),
                MenuItem::action(t("Save", "Enregistrer"), Save),
                MenuItem::action(t("Save As…", "Enregistrer sous…"), SaveAs),
            ],
        },
        Menu {
            name: t("Edit", "Édition").into(),
            items: vec![
                MenuItem::action(t("Undo", "Annuler"), Undo),
                MenuItem::action(t("Redo", "Rétablir"), Redo),
                MenuItem::separator(),
                MenuItem::action(t("Cut", "Couper"), Cut),
                MenuItem::action(t("Copy", "Copier"), Copy),
                MenuItem::action(t("Paste", "Coller"), Paste),
                MenuItem::action(t("Paste Values", "Coller les valeurs"), PasteValues),
                MenuItem::separator(),
                MenuItem::action(t("Select All", "Sélectionner tout"), SelectAll),
                MenuItem::action(t("Find…", "Rechercher…"), Find),
            ],
        },
        Menu {
            name: t("View", "Affichage").into(),
            items: vec![
                MenuItem::action(t("Zoom In", "Zoom avant"), ZoomIn),
                MenuItem::action(t("Zoom Out", "Zoom arrière"), ZoomOut),
                MenuItem::action(t("Actual Size", "Taille réelle"), ZoomReset),
            ],
        },
    ]);
}

/// Opt-in (`OPENSHEET_OFFICE_FONTS=1`): use the Aptos / Calibri fonts of a
/// Microsoft Office installation on this machine so cells render exactly as
/// in Excel. Nothing is copied or redistributed; without it, cells fall back
/// to a system font of similar proportions.
fn load_office_fonts(cx: &mut App) {
    if std::env::var("OPENSHEET_OFFICE_FONTS").map(|v| v != "1").unwrap_or(true) {
        return;
    }
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    for app in ["Microsoft Excel", "Microsoft Word", "Microsoft PowerPoint"] {
        let dir = format!("/Applications/{app}.app/Contents/Resources/DFonts");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if (name.starts_with("aptos") || name.starts_with("calibri")) && name.ends_with(".ttf") {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    fonts.push(bytes.into());
                }
            }
        }
        if !fonts.is_empty() {
            break;
        }
    }
    if !fonts.is_empty() {
        let _ = cx.text_system().add_fonts(fonts);
    }
}

fn main() {
    let demo = std::env::args().any(|a| a == "--demo");
    let file_arg = std::env::args().skip(1).find(|a| !a.starts_with("--"));

    Application::new().with_assets(icons::Assets).run(move |cx: &mut App| {
        i18n::set_lang(i18n::detect());
        load_office_fonts(cx);
        bind_keys(cx);
        set_menus(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let file_arg = file_arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("OpenSheet".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(13.), px(12.))),
                }),
                window_min_size: Some(size(px(640.), px(420.))),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    let mut app = ExcelApp::new(window, cx);
                    if demo {
                        demo::fill(&mut app, cx);
                    }
                    if let Some(path) = &file_arg {
                        app.load_path(std::path::PathBuf::from(path), cx);
                    }
                    app
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
