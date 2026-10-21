//! The themes interface driven in GPUI's headless window: the gallery,
//! the menus, the dialogs, the pickers. What these tests cannot tell is
//! how things look.

use super::*;
use crate::app::{ColorTarget, PopupKind};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::protect::SheetProtection;
use opensheet_engine::style::{Color, ThemeFont, ThemeSlot};
use opensheet_engine::{Rect, Style, Workbook};

/// A window on an empty workbook, with the key bindings of the application.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

/// Click something a view named for the tests.
fn press(cx: &mut VisualTestContext, name: &str) {
    let name: &'static str = Box::leak(name.to_string().into_boxed_str());
    let at = cx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{name} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

fn read<T>(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&Workbook) -> T,
) -> T {
    app.update(cx, |app, _| f(&app.wb))
}

fn style(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Style {
    let at = Rect::parse(name).unwrap();
    read(app, cx, |wb| wb.style_at(0, at.r0, at.c0).clone())
}

fn undo(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
}

/// The labels of the menu that is open, with whether each is ticked.
fn menu(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    app.update(cx, |app, _| {
        match app.popup.as_ref().map(|popup| &popup.kind) {
            Some(PopupKind::Menu(entries)) => entries
                .iter()
                .map(|entry| (entry.label.to_string(), entry.checked))
                .collect(),
            _ => Vec::new(),
        }
    })
}

fn popup_open(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> bool {
    app.update(cx, |app, _| app.popup.is_some())
}

#[gpui::test]
fn the_gallery_gives_the_workbook_a_theme(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // A fill picked among the theme colours, one among the standard ones.
    let accent = read(&app, cx, |wb| wb.theme().accent(1, 0.4));
    app.update(cx, |app, cx| {
        app.pick_color(ColorTarget::Fill, Some(accent), cx);
        app.select_cell(1, 0, cx);
        app.pick_color(ColorTarget::Fill, Some(Color(0xFFC000)), cx);
        app.select_cell(4, 4, cx);
    });
    let depth = read(&app, cx, |wb| wb.undo_label().map(str::to_string));

    run(&app, cx, Cmd::Themes);
    assert!(popup_open(&app, cx));
    press(cx, "theme:Facet");
    assert!(!popup_open(&app, cx));
    let facet = Theme::builtin("Facet").unwrap();
    assert_eq!(read(&app, cx, |wb| wb.theme().clone()), facet);
    assert_eq!(style(&app, cx, "A1").fill, Some(facet.accent(1, 0.4)));
    assert_eq!(style(&app, cx, "A2").fill, Some(Color(0xFFC000)));
    assert_eq!(&*style(&app, cx, "C3").font.name, "Trebuchet MS");
    assert_eq!(
        read(&app, cx, |wb| wb.undo_label().map(str::to_string)).as_deref(),
        Some("Theme")
    );
    // The pickers of the window now offer the colours of this theme.
    assert_eq!(crate::theme::palette_rows()[0][4], facet.accent(1, 0.0));

    // One step back.
    undo(&app, cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().clone()), Theme::office());
    assert_eq!(style(&app, cx, "A1").fill, Some(accent));
    assert_eq!(&*style(&app, cx, "C3").font.name, "Aptos Narrow");
    assert_eq!(
        read(&app, cx, |wb| wb.undo_label().map(str::to_string)),
        depth
    );
    assert_eq!(
        crate::theme::palette_rows()[0][4],
        Theme::office().accent(1, 0.0)
    );
}

#[gpui::test]
fn the_colors_and_fonts_menus(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let accent = read(&app, cx, |wb| wb.theme().accent(2, 0.0));
    app.update(cx, |app, cx| {
        app.pick_color(ColorTarget::Font, Some(accent), cx)
    });

    run(&app, cx, Cmd::Colors);
    press(cx, "colors:Blue Warm");
    let theme = read(&app, cx, |wb| wb.theme().clone());
    assert_eq!(
        (theme.name.as_str(), theme.colors.name.as_str()),
        ("Office Theme", "Blue Warm")
    );
    assert_eq!(
        style(&app, cx, "A1").font.color.map(|c| c.value()),
        Some(0x629DD1)
    );
    assert_eq!(
        read(&app, cx, |wb| wb.undo_label().map(str::to_string)).as_deref(),
        Some("Theme Colors")
    );

    run(&app, cx, Cmd::Fonts);
    press(cx, "fonts:Arial");
    let theme = read(&app, cx, |wb| wb.theme().clone());
    assert_eq!(
        (&*theme.fonts.major, &*theme.fonts.minor),
        ("Arial", "Arial")
    );
    assert_eq!(&*style(&app, cx, "D4").font.name, "Arial");
    // The default column is as wide as eight digits of the new font.
    let width = read(&app, cx, |wb| wb.sheets[0].default_col_width);
    assert_eq!(width, read(&app, cx, |wb| wb.font_metrics().col_width));
    assert_eq!(width, 72.0);

    undo(&app, cx);
    undo(&app, cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().clone()), Theme::office());
    assert_eq!(read(&app, cx, |wb| wb.sheets[0].default_col_width), 64.0);
}

#[gpui::test]
fn the_effects_menu_keeps_the_choice(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::Effects);
    let entries = menu(&app, cx);
    assert_eq!(entries.len(), 15);
    assert_eq!(entries[0], ("Office".to_string(), true));
    assert_eq!(entries[14], ("Glossy".to_string(), false));
    app.update_in(cx, |app, window, cx| {
        let action = match app.popup.take().map(|popup| popup.kind) {
            Some(PopupKind::Menu(entries)) => entries[14].action.clone().unwrap(),
            _ => panic!("the menu is open"),
        };
        action(app, window, cx);
    });
    redraw(cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().effects.clone()), "Glossy");
    run(&app, cx, Cmd::Effects);
    assert!(menu(&app, cx)[14].1 && !menu(&app, cx)[0].1);
    app.update(cx, |app, _| app.popup = None);
    undo(&app, cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().effects.clone()), "Office");
}

#[gpui::test]
fn the_customize_dialogs_make_sets_of_ones_own(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::Colors);
    press(cx, "command:CustomizeColors");
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 1);
    // The name is proposed; Return keeps the set and gives it to the workbook.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 0);
    let theme = read(&app, cx, |wb| wb.theme().clone());
    assert_eq!(theme.colors.name, "Custom 1");
    assert!(theme.colors.same_colors(&Theme::office().colors));
    assert_eq!(custom_sets().colors.len(), 1);
    // It is in the menu from now on, and the next one is "Custom 2".
    run(&app, cx, Cmd::Colors);
    assert!(cx.debug_bounds("colors:Custom 1").is_some());
    press(cx, "command:CustomizeColors");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 0);
    assert_eq!(custom_sets().colors.len(), 1);
    // The set is the application's: it is still listed once another
    // workbook is in the window, and it is kept in the settings file.
    app.update(cx, |app, cx| {
        let wb = opensheet_engine::Workbook::new(opensheet_engine::Locale::en());
        app.set_workbook(wb, None, "Other".to_string(), cx);
    });
    redraw(cx);
    run(&app, cx, Cmd::Colors);
    assert!(cx.debug_bounds("colors:Custom 1").is_some());
    cx.simulate_keystrokes("escape");
    redraw(cx);
    let kept = crate::features::preferences::settings::get();
    assert_eq!(kept.theme_colors.len(), 1);
    assert_eq!(
        crate::features::preferences::settings::Settings::from_text(&kept.to_text()).theme_colors,
        kept.theme_colors
    );

    run(&app, cx, Cmd::Fonts);
    press(cx, "command:CustomizeFonts");
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let theme = read(&app, cx, |wb| wb.theme().clone());
    assert_eq!(theme.fonts.name, "Custom 1");
    assert!(theme.fonts.same_fonts(&Theme::office().fonts));
    undo(&app, cx);
    undo(&app, cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().clone()), Theme::office());
}

#[gpui::test]
fn fonts_of_the_theme_in_the_font_list(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // The cells of a new workbook are in the body font.
    let labels = crate::theme::theme_font_labels(&Theme::office());
    assert_eq!(
        labels,
        [
            "Aptos Display (Headings)".to_string(),
            "Aptos Narrow (Body)".to_string()
        ]
    );
    assert_eq!(
        crate::theme::font_label(&style(&app, cx, "A1").font),
        "Aptos Narrow (Body)"
    );

    app.update(cx, |app, cx| {
        app.set_font_name("Aptos Display (Headings)", cx);
        app.select_cell(1, 0, cx);
        app.set_font_name("Georgia", cx);
        app.select_cell(2, 0, cx);
        // The same font picked by its own name is not the font of the theme.
        app.set_font_name("Aptos Narrow", cx);
    });
    assert_eq!(style(&app, cx, "A1").font.theme, Some(ThemeFont::Major));
    assert_eq!(style(&app, cx, "A2").font.theme, None);
    assert_eq!(style(&app, cx, "A3").font.theme, None);

    app.update(cx, |app, cx| {
        set_theme(app, Theme::builtin("Wood Type").unwrap(), cx)
    });
    redraw(cx);
    assert_eq!(&*style(&app, cx, "A1").font.name, "Rockwell Condensed");
    assert_eq!(&*style(&app, cx, "A2").font.name, "Georgia");
    assert_eq!(&*style(&app, cx, "A3").font.name, "Aptos Narrow");
    assert_eq!(&*style(&app, cx, "A4").font.name, "Rockwell");
    assert_eq!(
        crate::theme::font_label(&style(&app, cx, "A1").font),
        "Rockwell Condensed (Headings)"
    );
    // What the ribbon lists first is the fonts of the theme of now.
    redraw(cx);
    assert_eq!(
        crate::theme::theme_font_labels(&crate::theme::document_theme())[1],
        "Rockwell (Body)"
    );
}

#[gpui::test]
fn a_protected_sheet_keeps_its_theme(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
        app.commit("protect", cx);
    });
    redraw(cx);
    for cmd in [
        Cmd::Themes,
        Cmd::Colors,
        Cmd::Fonts,
        Cmd::Effects,
        Cmd::CustomizeColors,
        Cmd::CustomizeFonts,
        Cmd::Browse,
    ] {
        assert!(!app.update(cx, |app, _| cmd.enabled(app)), "{cmd:?}");
    }
    assert!(app.update(cx, |app, _| Cmd::SaveCurrent.enabled(app)));
    app.update(cx, |app, cx| {
        set_theme(app, Theme::builtin("Facet").unwrap(), cx)
    });
    redraw(cx);
    assert_eq!(read(&app, cx, |wb| wb.theme().clone()), Theme::office());
    // The refusal is said.
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 1);
}

#[gpui::test]
fn theme_files_are_saved_and_browsed_for(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let path =
        std::env::temp_dir().join(format!("opensheet-theme-test-{}.thmx", std::process::id()));
    let mut colors = Theme::office().colors;
    colors.name = "Mine".to_string();
    colors.set(ThemeSlot::Accent1, 0xAA0044);
    app.update(cx, |app, cx| {
        set_colors(app, colors.clone(), cx);
        save_theme_file(app, path.clone(), cx);
    });
    redraw(cx);
    assert!(path.exists());
    // Another workbook takes the theme from the file: it has the name of
    // the file and the colours that were saved.
    let (other, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    other.update(cx, |app, cx| load_theme_file(app, path.clone(), cx));
    redraw(cx);
    let theme = read(&other, cx, |wb| wb.theme().clone());
    assert!(theme.name.starts_with("opensheet-theme-test-"));
    assert_eq!(
        (theme.colors.name.as_str(), theme.rgb(ThemeSlot::Accent1)),
        ("Mine", 0xAA0044)
    );
    assert_eq!(
        read(&other, cx, |wb| wb.undo_label().map(str::to_string)).as_deref(),
        Some("Theme")
    );
    // The colours of a file are listed as its own.
    let listed = color_choices(&theme, &[]);
    assert!(listed[0].custom && listed[0].current);
    std::fs::remove_file(&path).ok();
    // A file that is no theme is refused, and says so.
    other.update(cx, |app, cx| load_theme_file(app, path.clone(), cx));
    redraw(cx);
    assert_eq!(other.update(cx, |app, _| app.modals.len()), 1);
}
