//! The Preferences window: the page of panes, and the panes whose
//! settings exist in this application (General, View, Edit, AutoCorrect,
//! Calculation, Error Checking, Custom Lists, Tables & Filters). The
//! other panes of Excel are on the page, greyed out: nothing of theirs is
//! shown that would do nothing. AutoCorrect and Custom Lists, which hold
//! lists, are in `pane_autocorrect.rs` and `pane_lists.rs`.
//!
//! Like Excel's, the window has no OK: a box or a choice takes effect at
//! once, a typed value when the pane is left or the window closed.

use super::settings::{self, Settings};
use crate::app::ExcelApp;
use crate::features::misc;
use crate::features::{Command, Cx};
use crate::i18n::t;
use crate::shell::{Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::widgets::text_area::{AreaEvent, TextArea};
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use crate::widgets::{self, Button, Checkbox, GroupBox, NumberField, RadioGroup, Select, TextField, dialog, label};
use gpui::{AnyElement, App, Context, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use opensheet_engine::ops::Direction;
use opensheet_engine::{CalcMode, IterativeCalc};

/// A pane of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pane {
    General,
    View,
    Edit,
    AutoCorrect,
    Calculation,
    ErrorChecking,
    CustomLists,
    Tables,
}

impl Pane {
    fn title(self) -> &'static str {
        match self {
            Pane::General => t("General", "Général"),
            Pane::View => crate::i18n::t_ctx("preferences", "View", "Affichage"),
            Pane::Edit => crate::i18n::t_ctx("preferences", "Edit", "Modifier"),
            Pane::AutoCorrect => t("AutoCorrect", "Correction automatique"),
            Pane::CustomLists => t("Custom Lists", "Listes personnalisées"),
            Pane::Tables => t("Tables & Filters", "Tableaux et filtres"),
            Pane::Calculation => t("Calculation", "Calcul"),
            Pane::ErrorChecking => t("Error Checking", "Vérification des erreurs"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Pane::General => "format-cells",
            Pane::View => "eye",
            Pane::Edit => "ink-pencil",
            Pane::AutoCorrect => "spell",
            Pane::CustomLists => "sort-asc",
            Pane::Tables => "table",
            Pane::Calculation => "calculator",
            Pane::ErrorChecking => "misc-error-checking",
        }
    }
}

/// The page of panes, as Excel groups them: a pane this application has,
/// or the name of one it has not.
fn page() -> Vec<(&'static str, Vec<Result<Pane, &'static str>>)> {
    vec![
        (
            t("Authoring", "Création"),
            vec![
                Ok(Pane::General),
                Ok(Pane::View),
                Ok(Pane::Edit),
                Ok(Pane::AutoCorrect),
                Err(t("Chart", "Graphique")),
            ],
        ),
        (
            t("Formulas and Lists", "Formules et listes"),
            vec![
                Ok(Pane::Calculation),
                Ok(Pane::ErrorChecking),
                Ok(Pane::CustomLists),
                Err(t("AutoComplete", "Saisie semi-automatique")),
                Ok(Pane::Tables),
            ],
        ),
        (
            t("Sharing and Privacy", "Partage et confidentialité"),
            vec![
                Err(t("Save", "Enregistrer")),
                Err(t("Compatibility", "Compatibilité")),
                Err(t("Security", "Sécurité")),
                Err(t("Ribbon & Toolbar", "Ruban et barre d'outils")),
            ],
        ),
    ]
}

/// What the window shows of the workbook and of the window behind it,
/// read when it opens (it cannot look at the application while it is
/// being built) and kept in step as its controls change them.
pub(super) struct Shown {
    pub(super) settings: Settings,
    calc_mode: CalcMode,
    iterative: IterativeCalc,
    formula_bar: bool,
    gridlines: bool,
    headings: bool,
    formulas: bool,
    outline_symbols: bool,
    /// The fonts of this machine, for "Default font".
    fonts: Vec<String>,
    /// The font of a new workbook when none is chosen.
    engine_font: String,
    /// The custom lists nobody can change, in the language of the
    /// workbook.
    pub(super) builtin_lists: Vec<Vec<String>>,
    /// The sheet in front and its name: where "Import list from cells"
    /// reads.
    pub(super) sheet: usize,
    pub(super) sheet_name: String,
}

fn shown(app: &ExcelApp) -> Shown {
    let sheet = &app.wb.sheets[app.sheet];
    Shown {
        settings: settings::get(),
        calc_mode: app.wb.calc_mode(),
        iterative: app.wb.iterative_calc(),
        formula_bar: app.show_formula_bar,
        gridlines: sheet.show_gridlines,
        headings: sheet.show_headers,
        formulas: misc::shows_formulas(app),
        outline_symbols: sheet.outline.show_symbols,
        fonts: app.fonts.names.clone(),
        engine_font: opensheet_engine::Style::default().font.name.to_string(),
        builtin_lists: opensheet_engine::Workbook::builtin_lists(app.wb.locale),
        sheet: app.sheet,
        sheet_name: sheet.name.clone(),
    }
}

pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    open_on(app, None, window, cx);
}

/// Open the window on the page of panes (`None`) or on one pane.
pub(super) fn open_on(app: &mut ExcelApp, pane: Option<Pane>, window: &mut Window, cx: &mut Cx) -> Entity<PreferencesWindow> {
    app.popup = None;
    let state = shown(app);
    app.open_modal(window, cx, |modal, _, cx| PreferencesWindow::new(modal, state, pane, cx))
}

pub(super) struct PreferencesWindow {
    pub(super) modal: Modal,
    pub(super) pane: Option<Pane>,
    pub(super) shown: Shown,
    /// AutoCorrect: the two boxes over the list.
    pub(super) replace_from: Entity<TextInput>,
    pub(super) replace_with: Entity<TextInput>,
    /// Custom Lists: the list chosen (0 is "NEW LIST", then the built-in
    /// ones, then the user's), its entries, and the range to import.
    pub(super) list_at: usize,
    pub(super) list_entries: Entity<TextArea>,
    pub(super) import_range: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
    pub(super) user_name: Entity<TextInput>,
    pub(super) font_size: Entity<TextInput>,
    pub(super) sheets: Entity<TextInput>,
    pub(super) places: Entity<TextInput>,
    pub(super) max_iterations: Entity<TextInput>,
    pub(super) max_change: Entity<TextInput>,
}

const DIRECTIONS: [Direction; 4] = [Direction::Down, Direction::Right, Direction::Up, Direction::Left];

fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => t("Down", "Bas"),
        Direction::Right => t("Right", "Droite"),
        Direction::Up => t("Up", "Haut"),
        Direction::Left => t("Left", "Gauche"),
    }
}

const MODES: [CalcMode; 3] = [CalcMode::Automatic, CalcMode::AutomaticExceptTables, CalcMode::Manual];

impl PreferencesWindow {
    pub(super) fn new(modal: Modal, shown: Shown, pane: Option<Pane>, cx: &mut Context<Self>) -> Self {
        let s = &shown.settings;
        let replace_from = widgets::input(cx, "");
        let replace_with = widgets::input(cx, "");
        // The list follows what is typed over it, and so do its buttons.
        let redraw = |_: &mut Self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Changed) {
                cx.notify();
            }
        };
        let list_entries = cx.new(|cx| {
            let mut area = TextArea::field(cx);
            area.min_rows = 9;
            area.max_height = Some(190.0);
            area
        });
        let subscriptions = vec![
            cx.subscribe(&replace_from, redraw),
            cx.subscribe(&replace_with, redraw),
            cx.subscribe(&list_entries, |_, _, event: &AreaEvent, cx| {
                if matches!(event, AreaEvent::Changed) {
                    cx.notify();
                }
            }),
        ];
        PreferencesWindow {
            import_range: modal.ref_input(RefStyle::absolute(), "", cx),
            modal,
            pane,
            replace_from,
            replace_with,
            list_at: 0,
            list_entries,
            _subscriptions: subscriptions,
            user_name: widgets::input(cx, s.user_name.clone()),
            font_size: widgets::number_input(cx, Some(s.default_font_size as f64), 1.0, 409.0, 1.0),
            sheets: widgets::number_input(cx, Some(s.sheets_in_new_workbook as f64), 1.0, 255.0, 1.0),
            places: widgets::number_input(cx, Some(s.decimal_places as f64), -300.0, 300.0, 1.0),
            max_iterations: widgets::number_input(cx, Some(shown.iterative.max_iterations as f64), 1.0, 32767.0, 1.0),
            max_change: widgets::number_input(cx, Some(shown.iterative.max_change), 0.0, f64::MAX, 0.001),
            shown,
        }
    }

    /// Change a setting of the application.
    pub(super) fn set(&mut self, cx: &mut Context<Self>, change: impl Fn(&mut Settings)) {
        change(&mut self.shown.settings);
        settings::update(change);
        // The grid and the window follow (the green corners, the tabs).
        // The workbook takes its part of them (lists, tables).
        self.modal.update_app(cx, |app, cx| {
            super::apply_to_workbook(&mut app.wb);
            cx.notify();
        });
        cx.notify();
    }

    /// Store the iterative calculation settings in the workbook.
    fn set_iterative(&mut self, cx: &mut Context<Self>) {
        let iterative = self.shown.iterative;
        self.modal.update_app(cx, |app, cx| {
            if app.wb.iterative_calc() != iterative {
                // The workbook calculates its circular references again.
                app.wb.set_iterative_calc(iterative);
                app.refresh(cx);
            }
        });
        cx.notify();
    }

    /// Take what was typed into the fields: each value that makes sense
    /// is stored, the others are put back as they were.
    pub(super) fn commit_fields(&mut self, cx: &mut Context<Self>) {
        let number = |field: &Entity<TextInput>, cx: &App| field.read(cx).number();
        let user_name = self.user_name.read(cx).text().trim().to_string();
        let font_size = number(&self.font_size, cx).filter(|size| (1.0..=409.0).contains(size));
        let sheets = number(&self.sheets, cx).filter(|count| (1.0..=255.0).contains(count) && count.fract() == 0.0);
        let places = number(&self.places, cx).filter(|places| (-300.0..=300.0).contains(places) && places.fract() == 0.0);
        self.set(cx, |settings| {
            settings.user_name = user_name.clone();
            if let Some(size) = font_size {
                settings.default_font_size = size as f32;
            }
            if let Some(count) = sheets {
                settings.sheets_in_new_workbook = count as u32;
            }
            if let Some(places) = places {
                settings.decimal_places = places as i32;
            }
        });
        let iterations = number(&self.max_iterations, cx).filter(|n| (1.0..=32767.0).contains(n) && n.fract() == 0.0);
        let change = number(&self.max_change, cx).filter(|change| change.is_finite() && *change >= 0.0);
        if let Some(iterations) = iterations {
            self.shown.iterative.max_iterations = iterations as u32;
        }
        if let Some(change) = change {
            self.shown.iterative.max_change = change;
        }
        self.set_iterative(cx);
        // What was refused shows its stored value again.
        let sep = crate::i18n::lang().locale().decimal_sep;
        let (s, iterative) = (self.shown.settings.clone(), self.shown.iterative);
        let fields = [
            (&self.font_size, s.default_font_size as f64),
            (&self.sheets, s.sheets_in_new_workbook as f64),
            (&self.places, s.decimal_places as f64),
            (&self.max_iterations, iterative.max_iterations as f64),
            (&self.max_change, iterative.max_change),
        ];
        for (field, value) in fields {
            let text = widgets::number_text(value, sep);
            field.update(cx, |field, cx| {
                if field.text() != text {
                    field.set_text(text, cx);
                }
            });
        }
    }

    pub(super) fn go(&mut self, pane: Option<Pane>, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        self.pane = pane;
        cx.notify();
    }

    fn close(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        self.modal.close(window, cx);
    }

    pub(super) fn set_calc_mode(&mut self, mode: CalcMode, cx: &mut Context<Self>) {
        self.shown.calc_mode = mode;
        self.modal.update_app(cx, |app, cx| {
            app.wb.set_calc_mode(mode);
            // Back to automatic: what waited is calculated.
            if mode != CalcMode::Manual {
                app.wb.calculate_now();
            }
            app.refresh(cx);
        });
        cx.notify();
    }

    /// "Outline symbols" of the sheet in front: the flag of its outline,
    /// as one undo step.
    pub(super) fn set_outline_symbols(&mut self, on: bool, cx: &mut Context<Self>) {
        self.shown.outline_symbols = on;
        self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            let outline = &app.wb.sheets[sheet].outline;
            if outline.show_symbols != on {
                let (below, right) = (outline.summary_below, outline.summary_right);
                app.wb.set_outline_settings(sheet, below, right, on);
                app.commit(t("Outline symbols", "Symboles du plan"), cx);
            }
        });
        cx.notify();
    }

    pub(super) fn set_iteration(&mut self, on: bool, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        self.shown.iterative.enabled = on;
        self.set_iterative(cx);
    }

    // ---- the page -------------------------------------------------------

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let tile = |entry: Result<Pane, &'static str>, cx: &mut Context<Self>| -> AnyElement {
            let (name, icon_name, pane) = match entry {
                Ok(pane) => (pane.title(), pane.icon(), Some(pane)),
                Err(name) => (name, "more", None),
            };
            let color = if pane.is_some() { theme::TEXT } else { theme::TEXT_DISABLED };
            div()
                .id(SharedString::from(format!("prefs-{name}")))
                .w(px(104.))
                .h(px(70.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .rounded(px(6.))
                .text_size(px(11.5))
                .text_color(c(color))
                .child(icon(icon_name, 26.0, color))
                .child(div().text_center().child(name))
                .map(|tile| match pane {
                    Some(pane) => tile
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .on_click(cx.listener(move |this, _, _, cx| this.go(Some(pane), cx))),
                    None => tile.tooltip(tooltip(t("Not available in OpenSheet", "Non disponible dans OpenSheet"))),
                })
                .into_any_element()
        };
        let mut column = div().flex().flex_col().gap(px(10.));
        for (title, entries) in page() {
            let tiles: Vec<AnyElement> = entries.into_iter().map(|entry| tile(entry, cx)).collect();
            column = column.child(GroupBox::new(title).child(div().flex().flex_wrap().gap(px(4.)).children(tiles)));
        }
        column.into_any_element()
    }

    // ---- the panes ------------------------------------------------------

    pub(super) fn check(
        &self,
        id: &'static str,
        text: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, bool, &mut Window, &mut Context<Self>) + 'static,
    ) -> Checkbox {
        Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, window, cx| change(this, *on, window, cx)))
    }

    fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shown.settings;
        // The font of the engine first, then those of the machine.
        let mut fonts: Vec<SharedString> = vec![self.shown.engine_font.clone().into()];
        fonts.extend(self.shown.fonts.iter().filter(|name| **name != self.shown.engine_font).map(|name| SharedString::from(name.clone())));
        let chosen = if s.default_font.is_empty() { Some(0) } else { fonts.iter().position(|name| name.as_ref() == s.default_font) };
        let names = fonts.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                GroupBox::new(t("Personalize", "Personnaliser"))
                    .child(row(t("User name:", "Nom d'utilisateur :"), TextField::new(&self.user_name).width(240.0))),
            )
            .child(
                GroupBox::new(t("Settings", "Paramètres"))
                    .child(row(
                        t("Default font:", "Police par défaut :"),
                        Select::new("prefs-font", fonts, chosen).width(240.0).on_change(cx.listener(move |this, index: &usize, _, cx| {
                            // The first entry is the engine's own font.
                            let name = if *index == 0 { String::new() } else { names[*index].to_string() };
                            this.set(cx, |settings| settings.default_font = name.clone());
                        })),
                    ))
                    .child(row(t("Font size:", "Taille de la police :"), NumberField::new("prefs-font-size", &self.font_size).width(70.0)))
                    .child(row(
                        t("Sheets in new workbooks:", "Feuilles dans les nouveaux classeurs :"),
                        NumberField::new("prefs-sheets", &self.sheets).width(70.0),
                    )),
            )
            .child(
                // Where the scripts of the Automate tab are kept.
                GroupBox::new(t("Office Scripts", "Scripts Office"))
                    .child(row(
                        t("Scripts folder:", "Dossier des scripts :"),
                        div()
                            .w(px(240.))
                            .truncate()
                            .text_color(c(theme::TEXT_MUTED))
                            .child(crate::features::automate::scripts_folder().to_string_lossy().to_string()),
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                Button::new("prefs-scripts-default", t("Use Default", "Par défaut"))
                                    .disabled(s.scripts_folder.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.set_scripts_folder(None, cx))),
                            )
                            .child(Button::new("prefs-scripts-choose", t("Choose…", "Choisir…")).on_click(cx.listener(|this, _, _, cx| this.choose_scripts_folder(cx)))),
                    ),
            )
            .into_any_element()
    }

    /// Keep the scripts in another folder (`None`: the application's own).
    pub(super) fn set_scripts_folder(&mut self, folder: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        let text = folder.map(|folder| folder.to_string_lossy().to_string()).unwrap_or_default();
        self.set(cx, |settings| settings.scripts_folder = text.clone());
    }

    /// Ask for the folder of the scripts.
    fn choose_scripts_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.set_scripts_folder(paths.into_iter().next(), cx)).ok();
            }
        })
        .detach();
    }

    fn render_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.shown;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                GroupBox::new(t("Show in Workbook", "Afficher dans le classeur"))
                    .child(self.check("prefs-formula-bar", t("Formula bar", "Barre de formule"), shown.formula_bar, cx, |this, on, _, cx| {
                        this.shown.formula_bar = on;
                        this.modal.update_app(cx, |app, cx| {
                            app.show_formula_bar = on;
                            cx.notify();
                        });
                        cx.notify();
                    }))
                    .child(self.check("prefs-sheet-tabs", t("Sheet tabs", "Onglets de feuille"), shown.settings.show_sheet_tabs, cx, |this, on, _, cx| {
                        this.set(cx, |settings| settings.show_sheet_tabs = on);
                    })),
            )
            .child(GroupBox::new(t("In Ribbon, Show", "Dans le ruban, afficher")).child(self.check(
                "prefs-developer",
                t("Developer tab", "Onglet Développeur"),
                shown.settings.developer_tab,
                cx,
                |this, on, _, cx| {
                    this.set(cx, |settings| settings.developer_tab = on);
                },
            )))
            .child(
                // These three belong to the sheet in front, as in Excel.
                GroupBox::new(t("Window options", "Options de fenêtre"))
                    .child(self.check("prefs-gridlines", t("Gridlines", "Quadrillage"), shown.gridlines, cx, |this, on, _, cx| {
                        this.shown.gridlines = on;
                        this.modal.update_app(cx, |app, cx| {
                            if app.wb.sheets[app.sheet].show_gridlines != on {
                                app.toggle_gridlines(cx);
                            }
                        });
                        cx.notify();
                    }))
                    .child(self.check(
                        "prefs-headings",
                        t("Row and column headers", "En-têtes des lignes et des colonnes"),
                        shown.headings,
                        cx,
                        |this, on, _, cx| {
                            this.shown.headings = on;
                            this.modal.update_app(cx, |app, cx| {
                                if app.wb.sheets[app.sheet].show_headers != on {
                                    app.toggle_headings(cx);
                                }
                            });
                            cx.notify();
                        },
                    ))
                    .child(self.check("prefs-formulas", t("Formulas", "Formules"), shown.formulas, cx, |this, on, window, cx| {
                        this.shown.formulas = on;
                        this.modal.update_app(cx, |app, cx| {
                            if misc::shows_formulas(app) != on {
                                misc::Cmd::ShowFormulas.run(app, window, cx);
                            }
                        });
                        cx.notify();
                    }))
                    .child(self.check(
                        "prefs-outline-symbols",
                        t("Outline symbols", "Symboles du plan"),
                        shown.outline_symbols,
                        cx,
                        |this, on, _, cx| this.set_outline_symbols(on, cx),
                    )),
            )
            .child(note(t(
                "Not available in OpenSheet: zero values, comment indicators, page breaks and the display of objects.",
                "Non disponible dans OpenSheet : valeurs zéro, indicateurs de commentaire, sauts de page et affichage des objets.",
            )))
            .into_any_element()
    }

    fn render_edit(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shown.settings;
        let directions: Vec<SharedString> = DIRECTIONS.iter().map(|direction| direction_label(*direction).into()).collect();
        let direction = DIRECTIONS.iter().position(|direction| *direction == s.move_direction);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                GroupBox::new(t("Edit Options", "Options d'édition"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(self.check(
                                "prefs-move",
                                t("After pressing Return, move selection:", "Déplacer la sélection après validation :"),
                                s.move_after_return,
                                cx,
                                |this, on, _, cx| this.set(cx, |settings| settings.move_after_return = on),
                            ))
                            .child(
                                Select::new("prefs-direction", directions, direction)
                                    .width(110.0)
                                    .disabled(!s.move_after_return)
                                    .on_change(cx.listener(|this, index: &usize, _, cx| {
                                        let direction = DIRECTIONS[*index];
                                        this.set(cx, |settings| settings.move_direction = direction);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(self.check(
                                "prefs-decimal",
                                t("Automatically insert a decimal point (places):", "Insérer automatiquement une décimale fixe (places) :"),
                                s.fixed_decimal,
                                cx,
                                |this, on, _, cx| {
                                    this.commit_fields(cx);
                                    this.set(cx, |settings| settings.fixed_decimal = on);
                                },
                            ))
                            .child(NumberField::new("prefs-places", &self.places).width(70.0)),
                    ),
            )
            .child(note(t(
                "Not available in OpenSheet: AutoComplete for cell values, the alert before overwriting cells, the date options and the automatic data conversions.",
                "Non disponible dans OpenSheet : la saisie semi-automatique des valeurs de cellule, l'alerte avant remplacement, les options de date et les conversions automatiques de données.",
            )))
            .into_any_element()
    }

    fn render_calculation(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.shown;
        let modes: Vec<SharedString> = vec![
            t("Automatic", "Automatique").into(),
            t("Automatic except for data tables", "Automatique sauf dans les tables de données").into(),
            t("Manual", "Manuel").into(),
        ];
        let mode = MODES.iter().position(|mode| *mode == shown.calc_mode);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                // The workbook's, kept in its file.
                GroupBox::new(t("Calculation Options", "Options de calcul"))
                    .child(RadioGroup::new("prefs-calc-mode", modes, mode).on_change(cx.listener(|this, index: &usize, _, cx| {
                        this.set_calc_mode(MODES[*index], cx);
                    })))
                    .child(self.check(
                        "prefs-recalc-save",
                        t("Recalculate workbook before saving", "Recalculer le classeur avant de l'enregistrer"),
                        shown.settings.recalc_before_save,
                        cx,
                        |this, on, _, cx| this.set(cx, |settings| settings.recalc_before_save = on),
                    )),
            )
            .child(
                GroupBox::new(t("Iteration", "Itération"))
                    .child(self.check(
                        "prefs-iterate",
                        t("Use iterative calculation", "Activer le calcul itératif"),
                        shown.iterative.enabled,
                        cx,
                        |this, on, _, cx| this.set_iteration(on, cx),
                    ))
                    .child(row(
                        t("Maximum iterations:", "Maximum d'itérations :"),
                        NumberField::new("prefs-max-iterations", &self.max_iterations).width(90.0),
                    ))
                    .child(row(t("Maximum change:", "Écart maximal :"), NumberField::new("prefs-max-change", &self.max_change).width(90.0))),
            )
            .child(note(t(
                "Not available in OpenSheet: the R1C1 reference style, the 1904 date system and precision as displayed.",
                "Non disponible dans OpenSheet : le style de référence L1C1, le calendrier depuis 1904 et la précision au format affiché.",
            )))
            .into_any_element()
    }

    fn render_error_checking(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shown.settings;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(GroupBox::new(Pane::ErrorChecking.title()).child(self.check(
                "prefs-background",
                t("Enable background error checking", "Activer la vérification des erreurs en arrière-plan"),
                s.background_error_checking,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.background_error_checking = on),
            )))
            .child(GroupBox::new(t("Rules", "Règles")).child(self.check(
                "prefs-rule-errors",
                t("Formulas that result in an error", "Formules générant des erreurs"),
                s.check_error_values,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.check_error_values = on),
            )))
            .child(note(t(
                "Excel's other rules (inconsistent formulas, numbers stored as text, formulas omitting cells, unlocked formulas, references to empty cells…) are not checked by OpenSheet.",
                "Les autres règles d'Excel (formules incohérentes, nombres stockés en tant que texte, formules omettant des cellules, formules non verrouillées, références à des cellules vides…) ne sont pas vérifiées par OpenSheet.",
            )))
            .into_any_element()
    }
}

impl PreferencesWindow {
    fn render_tables(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shown.settings;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                GroupBox::new(t("Tables", "Tableaux"))
                    .child(self.check(
                        "prefs-tables-expand",
                        t("Automatically expand tables", "Développer automatiquement les tableaux"),
                        s.tables_expand,
                        cx,
                        |this, on, _, cx| this.set(cx, |settings| settings.tables_expand = on),
                    ))
                    .child(self.check(
                        "prefs-tables-fill",
                        t("Automatically fill formulas", "Formules de remplissage automatique"),
                        s.tables_fill_formulas,
                        cx,
                        |this, on, _, cx| this.set(cx, |settings| settings.tables_fill_formulas = on),
                    )),
            )
            .child(note(t(
                "Not available in OpenSheet: turning off table names in formulas, and the grouping of dates in filters and PivotTables.",
                "Non disponible dans OpenSheet : la désactivation des noms de tableau dans les formules, et le regroupement des dates dans les filtres et les tableaux croisés dynamiques.",
            )))
            .into_any_element()
    }
}

/// A control after its label, the labels of a pane in one column.
fn row(text: &'static str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(230.)).flex_none().child(label(text)))
        .child(control)
}

/// What a pane says of the settings of Excel it does not have.
pub(super) fn note(text: &'static str) -> gpui::Div {
    div()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(c(theme::TEXT_MUTED))
        .child(text)
}

impl Render for PreferencesWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title: SharedString = match self.pane {
            Some(pane) => pane.title().into(),
            None => t("OpenSheet Preferences", "Préférences d'OpenSheet").into(),
        };
        let body = match self.pane {
            None => self.render_page(cx),
            Some(Pane::General) => self.render_general(cx),
            Some(Pane::View) => self.render_view(cx),
            Some(Pane::Edit) => self.render_edit(cx),
            Some(Pane::Calculation) => self.render_calculation(cx),
            Some(Pane::ErrorChecking) => self.render_error_checking(cx),
            Some(Pane::AutoCorrect) => self.render_autocorrect(cx),
            Some(Pane::CustomLists) => self.render_lists(cx),
            Some(Pane::Tables) => self.render_tables(cx),
        };
        let back = self.pane.is_some().then(|| {
            Button::new("prefs-show-all", t("Show All", "Tout afficher")).on_click(cx.listener(|this, _, _, cx| this.go(None, cx)))
        });
        dialog(&self.modal, title)
            .width(580.0)
            .on_ok(cx.listener(Self::close))
            .on_cancel(cx.listener(Self::close))
            .child(div().min_h(px(250.)).child(body))
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_between()
                    .child(div().children(back))
                    .child(Button::new("prefs-close", t("Close", "Fermer")).primary().on_click(cx.listener(Self::close))),
            )
    }
}
