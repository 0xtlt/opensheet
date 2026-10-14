//! OpenSheet — an open-source spreadsheet built with Rust and GPUI.

mod app;
mod app_windows;
mod chart_view;
mod demo;
mod features;
mod file;
mod grid;
mod i18n;
mod i18n_tables;
mod icons;
mod objects;
mod paths;
mod ribbon;
mod ribbon_fold;
mod shell;
mod text_input;
mod theme;
mod view;
mod widgets;

use app::*;
use futures::StreamExt;
use gpui::{App, Application, KeyBinding, Menu, MenuItem};
use i18n::t;

fn bind_keys(cx: &mut App) {
    text_input::bind_keys(cx);
    widgets::bind_keys(cx);
    features::comments::bind_keys(cx);
    features::misc::bind_keys(cx);
    features::edit_tools::bind_keys(cx);
    features::data_tools::bind_keys(cx);
    features::preferences::bind_keys(cx);
    features::proofing::bind_keys(cx);
    features::outline::bind_keys(cx);
    features::automate::bind_keys(cx);
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
        KeyBinding::new("alt-down", OpenDropDown, g),
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
        // Commands of the feature modules
        KeyBinding::new("cmd-1", FormatCells, w),
        KeyBinding::new("ctrl-1", FormatCells, w),
        KeyBinding::new("cmd-p", Print, w),
        KeyBinding::new("cmd-shift-l", ToggleFilter, w),
        KeyBinding::new("ctrl-shift-l", ToggleFilter, w),
        KeyBinding::new("cmd-t", InsertTable, w),
        KeyBinding::new("ctrl-t", InsertTable, w),
        KeyBinding::new("ctrl-l", InsertTable, w),
        KeyBinding::new("shift-f2", NewNote, w),
        KeyBinding::new("cmd-shift-m", NewComment, w),
        KeyBinding::new("cmd-shift-f2", NewComment, w),
        KeyBinding::new("ctrl-f3", NameManager, w),
        KeyBinding::new("cmd-k", InsertLink, w),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

pub(crate) fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "OpenSheet".into(),
            items: vec![
                MenuItem::action(t("Preferences…", "Préférences…"), features::preferences::OpenPreferences),
                MenuItem::separator(),
                MenuItem::action(t("Quit OpenSheet", "Quitter OpenSheet"), Quit),
            ],
        },
        Menu {
            name: t("File", "Fichier").into(),
            items: vec![
                MenuItem::action(t("New", "Nouveau"), NewWorkbook),
                MenuItem::action(t("Open…", "Ouvrir…"), Open),
                MenuItem::separator(),
                MenuItem::action(t("Save", "Enregistrer"), Save),
                MenuItem::action(t("Save As…", "Enregistrer sous…"), SaveAs),
                MenuItem::separator(),
                MenuItem::action(t("Print…", "Imprimer…"), Print),
                MenuItem::submenu(features::share::menu_bar()),
            ],
        },
        Menu {
            name: i18n::t_ctx("menu", "Edit", "Édition").into(),
            items: vec![
                MenuItem::action(t("Undo", "Annuler"), Undo),
                MenuItem::action(t("Redo", "Rétablir"), Redo),
                MenuItem::separator(),
                MenuItem::action(t("Cut", "Couper"), Cut),
                MenuItem::action(t("Copy", "Copier"), Copy),
                MenuItem::action(t("Paste", "Coller"), Paste),
                MenuItem::action(t("Paste Values", "Coller les valeurs"), PasteValues),
                MenuItem::action(t("Paste Special…", "Collage spécial…"), features::edit_tools::PasteSpecialDialog),
                MenuItem::separator(),
                MenuItem::action(t("Select All", "Sélectionner tout"), SelectAll),
                MenuItem::action(t("Find…", "Rechercher…"), Find),
                MenuItem::action(t("Replace…", "Remplacer…"), features::edit_tools::Replace),
            ],
        },
        Menu {
            name: i18n::t_ctx("menu", "Format", "Format").into(),
            items: vec![MenuItem::action(t("Cells…", "Cellule…"), FormatCells)],
        },
        Menu {
            name: i18n::t_ctx("menu", "View", "Affichage").into(),
            items: vec![
                MenuItem::action(t("Zoom In", "Zoom avant"), ZoomIn),
                MenuItem::action(t("Zoom Out", "Zoom arrière"), ZoomOut),
                MenuItem::action(t("Actual Size", "Taille réelle"), ZoomReset),
                MenuItem::separator(),
                MenuItem::action(t("Custom Views…", "Affichages personnalisés…"), features::custom_views::CustomViewsDialog),
                MenuItem::action(t("Macros…", "Macros…"), features::automate::ViewMacros),
            ],
        },
        features::help::menu_bar(),
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

/// `--export <path>`: write the workbook (a file given on the command line,
/// or the `--demo` sheet) to `path` without opening a window. The format
/// follows the extension (.xlsx, .csv, .tsv).
fn export(input: Option<&str>, demo: bool, output: &str) -> Result<(), String> {
    let locale = i18n::lang().locale();
    let mut wb = match input {
        Some(path) => opensheet_io::load(std::path::Path::new(path), locale).map_err(|e| e.to_string())?,
        None => opensheet_engine::Workbook::new(locale),
    };
    if demo {
        demo::fill_workbook(&mut wb);
    }
    opensheet_io::save(&wb, 0, std::path::Path::new(output)).map_err(|e| e.to_string())
}

/// Started by the system from an application bundle (macOS: the Finder,
/// the Dock, `open`), which brings the application to the front itself, or
/// leaves it behind when asked to (`open -g`). Started from a terminal, the
/// application has to come forward on its own.
fn launched_from_bundle() -> bool {
    cfg!(target_os = "macos")
        && std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("opensheet {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let demo = args.iter().any(|a| a == "--demo");
    // `--widgets`: open the gallery of the dialog toolkit at start.
    let gallery = args.iter().any(|a| a == "--widgets");
    let export_to = args.iter().position(|a| a == "--export").and_then(|i| args.get(i + 1)).cloned();
    // The files to open: how Windows and Linux pass the ones the user
    // double-clicked (each opens in a window of its own).
    let file_args: Vec<std::path::PathBuf> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--export"))
        .map(|(_, a)| std::path::PathBuf::from(a))
        .collect();

    if let Some(output) = export_to {
        i18n::set_lang(i18n::detect());
        let input = file_args.first().and_then(|p| p.to_str());
        match export(input, demo, &output) {
            Ok(()) => println!("{output}"),
            Err(err) => {
                eprintln!("opensheet: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    let app = Application::new().with_assets(icons::Assets);
    // macOS sends the files opened from the Finder or dropped on the Dock as
    // URLs: before the windows exist when they launched the application,
    // at any time after when it runs already.
    let (urls_sender, mut urls) = futures::channel::mpsc::unbounded::<Vec<String>>();
    app.on_open_urls(move |list| {
        let _ = urls_sender.unbounded_send(list);
    });
    app.run(move |cx: &mut App| {
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

        // The files of the command line and those the launch brought.
        let mut paths = file_args;
        while let Ok(list) = urls.try_recv() {
            paths.extend(app_windows::paths_of_urls(&list));
        }
        // Book1, unless files take its place.
        if paths.is_empty() || demo || gallery {
            app_windows::open_window(cx, move |app, window, cx| {
                if demo {
                    demo::fill(app, cx);
                }
                if gallery {
                    widgets::gallery::open(app, window, cx);
                }
            });
        }
        app_windows::open_paths(paths, cx);
        cx.spawn(async move |cx| {
            while let Some(list) = urls.next().await {
                let paths = app_windows::paths_of_urls(&list);
                if cx.update(|cx| app_windows::open_paths(paths, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        if !launched_from_bundle() {
            cx.activate(true);
        }
    });
}
