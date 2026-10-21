//! Document themes: the Themes gallery, the Colors, Fonts and Effects
//! menus of the Page Layout tab, and the dialogs that make a colour set
//! or a font pair of one's own.
//!
//! The theme itself is the engine's (`opensheet_engine::theme`): choosing
//! one here is `Workbook::set_theme` and a commit, one undo step that
//! recolours and refonts whatever refers to the theme. The drop-downs are
//! in `themes/gallery.rs`, the dialogs in `themes/dialogs.rs`, the names
//! Excel gives the built-in themes in each language in `themes/names.rs`.

mod dialogs;
mod gallery;
mod names;
#[cfg(test)]
mod ui_tests;

use super::preferences::settings;
use super::{Command, Cx, Feature, names as defined_names};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::shell::Alert;
use gpui::{PathPromptOptions, Window};
use opensheet_engine::theme::{Theme, ThemeColors, ThemeFonts, builtin};
use std::path::PathBuf;

pub use names::shown as shown_name;

pub struct Themes;

commands! {
    /// What the Themes group of the Page Layout tab offers.
    pub enum Cmd {
        Themes = "cell-styles", t("Themes", "Thèmes");
        Colors = "fill", t("Colors", "Couleurs");
        Fonts = "font-grow", t("Fonts", "Polices");
        Effects = "shapes", t("Effects", "Effets");
        CustomizeColors = "", t("Customize Colors…", "Personnaliser les couleurs…");
        CustomizeFonts = "", t("Customize Fonts…", "Personnaliser les polices…");
        Browse = "", t("Browse for Themes…", "Rechercher des thèmes…");
        SaveCurrent = "", t("Save Current Theme…", "Enregistrer le thème actif…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Themes => gallery::open(app, gallery::Kind::Themes, window, cx),
            Cmd::Colors => gallery::open(app, gallery::Kind::Colors, window, cx),
            Cmd::Fonts => gallery::open(app, gallery::Kind::Fonts, window, cx),
            Cmd::Effects => {
                let current = app.wb.theme().effects.clone();
                let entries = builtin::EFFECTS
                    .iter()
                    .map(|name| {
                        MenuEntry::new(shown_name(name), move |app, _, cx| {
                            set_effects(app, name, cx)
                        })
                        .checked(current.eq_ignore_ascii_case(name))
                    })
                    .collect();
                app.open_command_menu(entries, 220.0, window, cx);
            }
            Cmd::CustomizeColors => dialogs::open_colors(app, window, cx),
            Cmd::CustomizeFonts => dialogs::open_fonts(app, window, cx),
            Cmd::Browse => browse(app, cx),
            Cmd::SaveCurrent => save_current(app, cx),
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        // Excel greys the group out on a protected sheet; saving the theme
        // changes nothing.
        self == Cmd::SaveCurrent || defined_names::protected(&app.wb, app.sheet).is_none()
    }
}

impl Feature for Themes {
    fn name(&self) -> &'static str {
        "themes"
    }
}

/// The colour sets and font pairs made in the Customize dialogs, which
/// the menus list under "Custom". They are the application's, kept in
/// its settings file from one launch to the next.
#[derive(Default)]
pub struct ThemesUi {
    pub colors: Vec<ThemeColors>,
    pub fonts: Vec<ThemeFonts>,
}

/// The sets of the user's own.
pub fn custom_sets() -> ThemesUi {
    let settings = settings::get();
    ThemesUi {
        colors: settings.theme_colors,
        fonts: settings.theme_fonts,
    }
}

/// Keep `item` in a list of the user's own, in place of one of the same
/// name.
fn keep<T>(list: &mut Vec<T>, item: T, name_of: impl Fn(&T) -> &str) {
    let name = name_of(&item).to_string();
    match list
        .iter()
        .position(|other| name_of(other).eq_ignore_ascii_case(&name))
    {
        Some(at) => list[at] = item,
        None => list.push(item),
    }
}

/// Keep a colour set of the user's own.
pub fn keep_colors(colors: &ThemeColors) {
    settings::update(|settings| keep(&mut settings.theme_colors, colors.clone(), |set| &set.name));
}

/// Keep a font pair of the user's own.
pub fn keep_fonts(fonts: &ThemeFonts) {
    settings::update(|settings| keep(&mut settings.theme_fonts, fonts.clone(), |pair| &pair.name));
}

// ------------------------------------------------------------------ changes

/// Whether the theme may change: not while the sheet is protected.
fn may_change(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    match defined_names::protected(&app.wb, app.sheet) {
        Some(message) => {
            app.alert(Alert::new("OpenSheet", message).warning(), cx);
            false
        }
        None => true,
    }
}

/// Give the workbook another theme: one undo step.
pub fn set_theme(app: &mut ExcelApp, theme: Theme, cx: &mut Cx) {
    if may_change(app, cx) {
        app.wb.set_theme(theme);
        app.commit(t("Theme", "Thème"), cx);
    }
}

/// Give the theme other colours and leave its fonts and its name.
pub fn set_colors(app: &mut ExcelApp, colors: ThemeColors, cx: &mut Cx) {
    if may_change(app, cx) {
        app.wb.set_theme_colors(colors);
        app.commit(t("Theme Colors", "Couleurs du thème"), cx);
    }
}

/// Give the theme other fonts.
pub fn set_fonts(app: &mut ExcelApp, fonts: ThemeFonts, cx: &mut Cx) {
    if may_change(app, cx) {
        app.wb.set_theme_fonts(fonts);
        app.commit(t("Theme Fonts", "Polices du thème"), cx);
    }
}

/// Give the theme other effects. Nothing in this application is drawn
/// with them: the choice is kept and goes into the file.
pub fn set_effects(app: &mut ExcelApp, effects: &str, cx: &mut Cx) {
    if may_change(app, cx) {
        app.wb.set_theme_effects(effects);
        app.commit(t("Theme Effects", "Effets du thème"), cx);
    }
}

// ------------------------------------------------------------- what is listed

/// The themes of the gallery, in its order.
pub fn gallery_themes() -> Vec<Theme> {
    builtin::THEMES.iter().map(Theme::from).collect()
}

/// The built-in theme the workbook is in, if it is in one as it comes:
/// its place in the gallery.
pub fn current_theme(theme: &Theme) -> Option<usize> {
    builtin::THEMES.iter().position(|stock| {
        let stock = Theme::from(stock);
        stock.name == theme.name
            && stock.colors.same_colors(&theme.colors)
            && stock.fonts.same_fonts(&theme.fonts)
    })
}

/// A colour set of the Colors menu.
pub struct ColorChoice {
    pub colors: ThemeColors,
    /// One of the user's own, listed under "Custom".
    pub custom: bool,
    /// The colours the workbook has now.
    pub current: bool,
}

/// What the Colors menu lists: the user's own sets and, when they are
/// none of the others, the colours of the workbook (a file brings its
/// own); then Excel's.
pub fn color_choices(theme: &Theme, custom: &[ThemeColors]) -> Vec<ColorChoice> {
    let stock = Theme::builtin_colors();
    let mut own: Vec<ThemeColors> = custom.to_vec();
    let known =
        |set: &ThemeColors, list: &[ThemeColors]| list.iter().any(|other| other.same_colors(set));
    if !known(&theme.colors, &stock) && !known(&theme.colors, &own) {
        own.insert(0, theme.colors.clone());
    }
    // The first of the sets with these colours is the one that is ticked.
    let mut found = false;
    let mut choice = |colors: ThemeColors, custom: bool| {
        let current = !found && colors.same_colors(&theme.colors);
        found |= current;
        ColorChoice {
            colors,
            custom,
            current,
        }
    };
    let own: Vec<ColorChoice> = own.into_iter().map(|colors| choice(colors, true)).collect();
    own.into_iter()
        .chain(stock.into_iter().map(|colors| choice(colors, false)))
        .collect()
}

/// A font pair of the Fonts menu.
pub struct FontChoice {
    pub fonts: ThemeFonts,
    pub custom: bool,
    pub current: bool,
}

/// What the Fonts menu lists, as [`color_choices`] does for colours.
pub fn font_choices(theme: &Theme, custom: &[ThemeFonts]) -> Vec<FontChoice> {
    let stock = Theme::builtin_fonts();
    let mut own: Vec<ThemeFonts> = custom.to_vec();
    let known =
        |pair: &ThemeFonts, list: &[ThemeFonts]| list.iter().any(|other| other.same_fonts(pair));
    if !known(&theme.fonts, &stock) && !known(&theme.fonts, &own) {
        own.insert(0, theme.fonts.clone());
    }
    let mut found = false;
    let mut choice = |fonts: ThemeFonts, custom: bool| {
        let current = !found && fonts.same_fonts(&theme.fonts);
        found |= current;
        FontChoice {
            fonts,
            custom,
            current,
        }
    };
    let own: Vec<FontChoice> = own.into_iter().map(|fonts| choice(fonts, true)).collect();
    own.into_iter()
        .chain(stock.into_iter().map(|fonts| choice(fonts, false)))
        .collect()
}

/// The name the Customize dialogs propose for a new set: "Custom 1",
/// "Custom 2"... the first that is not taken.
pub fn new_custom_name(taken: &[String]) -> String {
    let base = t("Custom", "Personnalisé");
    (1..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.iter().any(|other| other.eq_ignore_ascii_case(name)))
        .unwrap_or_default()
}

// --------------------------------------------------------------- theme files

/// "Browse for Themes…": take the theme of a theme file (or of a workbook).
fn browse(app: &mut ExcelApp, cx: &mut Cx) {
    if !may_change(app, cx) {
        return;
    }
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = paths.await
            && let Some(path) = paths.into_iter().next()
        {
            this.update(cx, |app, cx| load_theme_file(app, path, cx))
                .ok();
        }
    })
    .detach();
}

/// Give the workbook the theme of the file at `path`.
pub fn load_theme_file(app: &mut ExcelApp, path: PathBuf, cx: &mut Cx) {
    let theme = std::fs::File::open(&path)
        .map_err(opensheet_io::IoError::from)
        .and_then(opensheet_io::read_thmx);
    match theme {
        Ok(theme) => set_theme(app, theme, cx),
        Err(err) => app.alert(
            Alert::new(
                t("Can't open this file", "Impossible d'ouvrir ce fichier"),
                format!("{}\n\n{}", path.display(), err),
            )
            .warning(),
            cx,
        ),
    }
}

/// "Save Current Theme…": write the theme of the workbook as a theme file.
fn save_current(app: &mut ExcelApp, cx: &mut Cx) {
    let dir = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Documents"))
        .unwrap_or_else(|| PathBuf::from("."));
    let name = format!("{}.thmx", app.wb.theme().name);
    let target = cx.prompt_for_new_path(&dir, Some(&name));
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(mut path))) = target.await {
            if path.extension().is_none() {
                path.set_extension("thmx");
            }
            this.update(cx, |app, cx| save_theme_file(app, path, cx))
                .ok();
        }
    })
    .detach();
}

/// Write the theme of the workbook to `path`, under the name of the file.
pub fn save_theme_file(app: &mut ExcelApp, path: PathBuf, cx: &mut Cx) {
    let mut theme = app.wb.theme().clone();
    if let Some(name) = path.file_stem() {
        theme.name = name.to_string_lossy().to_string();
    }
    let written = opensheet_io::write_thmx(&theme, std::io::Cursor::new(Vec::new()))
        .and_then(|bytes| Ok(std::fs::write(&path, bytes.into_inner())?));
    match written {
        Ok(()) => app.set_status(t("Saved", "Enregistré"), cx),
        Err(err) => app.alert(
            Alert::new(
                t(
                    "Can't save this file",
                    "Impossible d'enregistrer ce fichier",
                ),
                format!("{}\n\n{}", path.display(), err),
            )
            .warning(),
            cx,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_menus_list() {
        let office = Theme::office();
        assert_eq!(gallery_themes().len(), 29);
        assert_eq!(current_theme(&office), Some(0));
        assert_eq!(current_theme(&Theme::builtin("Facet").unwrap()), Some(2));

        // Excel's sets, the current one ticked.
        let colors = color_choices(&office, &[]);
        assert_eq!(colors.len(), 24);
        assert!(colors[0].current && !colors[0].custom);
        assert_eq!(colors.iter().filter(|c| c.current).count(), 1);
        // Other colours under the name of the theme: the theme is no
        // longer the one of the gallery, the set is ticked.
        let mut recoloured = office.clone();
        recoloured.colors = colors[5].colors.clone();
        assert_eq!(current_theme(&recoloured), None);
        assert!(color_choices(&recoloured, &[])[5].current);

        // Colours that are nobody's (a file's): listed first, as custom.
        let mut own = office.clone();
        own.colors.name = "Company".to_string();
        own.colors
            .set(opensheet_engine::style::ThemeSlot::Accent1, 0x123456);
        let listed = color_choices(&own, &[]);
        assert_eq!(listed.len(), 25);
        assert!(listed[0].custom && listed[0].current && listed[0].colors.name == "Company");
        // Once kept among the user's sets it is listed once.
        let listed = color_choices(&own, std::slice::from_ref(&own.colors));
        assert_eq!(
            (listed.len(), listed.iter().filter(|c| c.current).count()),
            (25, 1)
        );
        let listed = color_choices(&office, std::slice::from_ref(&own.colors));
        assert!(listed[0].custom && !listed[0].current && listed[1].current);

        let fonts = font_choices(&office, &[]);
        assert_eq!(fonts.len(), 26);
        assert!(fonts[0].current);
        let mut own = office.clone();
        own.fonts = ThemeFonts::new("Mine", "Georgia", "Verdana");
        let listed = font_choices(&own, &[]);
        assert!(listed[0].custom && listed[0].current && listed.len() == 27);

        assert_eq!(new_custom_name(&[]), "Custom 1");
        assert_eq!(
            new_custom_name(&["custom 1".to_string(), "Custom 3".to_string()]),
            "Custom 2"
        );
    }

    #[test]
    fn names_of_ones_own_are_shown_as_they_are() {
        // The tests run in English, where every name is itself; the other
        // languages are tables (see `names.rs` and `i18n_tables.rs`).
        assert_eq!(shown_name("Facet"), "Facet");
        assert_eq!(shown_name("Calibri-Cambria"), "Calibri-Cambria");
        assert_eq!(shown_name("Company & co"), "Company & co");
    }
}
