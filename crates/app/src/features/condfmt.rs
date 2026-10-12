//! Conditional formatting: the menus of the Home tab, the rule dialogs,
//! the rules manager, and what rules draw inside cells (data bars, icons).
//!
//! Not written yet: the commands are in place and say so. The grid
//! already paints cells with the style `Workbook::display` returns and
//! calls [`paint_adornment`] for the cells whose rules draw something.

use super::{Command, Cx, Feature};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{Draw, GridLayout};
use crate::i18n::t;
use gpui::{Window, WindowTextSystem};
use opensheet_engine::condfmt::Adornment;
use opensheet_engine::{Style, Value};

pub struct CondFormats;

commands! {
    /// What the user can ask of conditional formatting.
    pub enum Cmd {
        Menu = "cond-format", t("Conditional Formatting", "Mise en forme conditionnelle");
        GreaterThan = "", t("Greater Than…", "Supérieur à…");
        LessThan = "", t("Less Than…", "Inférieur à…");
        Between = "", t("Between…", "Entre…");
        EqualTo = "", t("Equal To…", "Égal à…");
        TextContains = "", t("Text that Contains…", "Texte qui contient…");
        DateOccurring = "", t("A Date Occurring…", "Une date se produisant…");
        DuplicateValues = "", t("Duplicate Values…", "Valeurs en double…");
        Top10Items = "", t("Top 10 Items…", "10 valeurs les plus élevées…");
        Top10Percent = "", t("Top 10 %…", "10 % de valeurs les plus élevées…");
        Bottom10Items = "", t("Bottom 10 Items…", "10 valeurs les moins élevées…");
        Bottom10Percent = "", t("Bottom 10 %…", "10 % de valeurs les moins élevées…");
        AboveAverage = "", t("Above Average…", "Valeurs supérieures à la moyenne…");
        BelowAverage = "", t("Below Average…", "Valeurs inférieures à la moyenne…");
        DataBars = "", t("More Data Bar Rules…", "Autres règles de barres de données…");
        ColorScales = "", t("More Color Scale Rules…", "Autres règles de nuances de couleurs…");
        IconSets = "", t("More Icon Set Rules…", "Autres règles de jeux d'icônes…");
        MoreRules = "", t("More Rules…", "Autres règles…");
        NewRule = "", t("New Rule…", "Nouvelle règle…");
        ClearSelected = "", t("Clear Rules from Selected Cells", "Effacer les règles des cellules sélectionnées");
        ClearSheet = "", t("Clear Rules from Entire Sheet", "Effacer les règles de la feuille entière");
        ClearTable = "", t("Clear Rules from This Table", "Effacer les règles de ce tableau");
        ClearPivot = "", t("Clear Rules from This PivotTable", "Effacer les règles de ce tableau croisé dynamique");
        Manage = "", t("Manage Rules…", "Gérer les règles…");
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
        match self {
            Cmd::Menu => {
                let entries = menu(app);
                app.open_command_menu(entries, 300.0, window, cx);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

/// The menu of the Conditional Formatting button.
fn menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let list = |commands: &[Cmd]| -> Vec<MenuEntry> { commands.iter().map(|cmd| MenuEntry::command(*cmd, app)).collect() };
    vec![
        MenuEntry::submenu(
            t("Highlight Cells Rules", "Règles de mise en surbrillance des cellules"),
            list(&[
                Cmd::GreaterThan,
                Cmd::LessThan,
                Cmd::Between,
                Cmd::EqualTo,
                Cmd::TextContains,
                Cmd::DateOccurring,
                Cmd::DuplicateValues,
                Cmd::MoreRules,
            ]),
        ),
        MenuEntry::submenu(
            t("Top/Bottom Rules", "Règles des valeurs de plage haute/basse"),
            list(&[
                Cmd::Top10Items,
                Cmd::Top10Percent,
                Cmd::Bottom10Items,
                Cmd::Bottom10Percent,
                Cmd::AboveAverage,
                Cmd::BelowAverage,
                Cmd::MoreRules,
            ]),
        ),
        MenuEntry::separator(),
        MenuEntry::submenu(t("Data Bars", "Barres de données"), list(&[Cmd::DataBars])),
        MenuEntry::submenu(t("Color Scales", "Nuances de couleurs"), list(&[Cmd::ColorScales])),
        MenuEntry::submenu(t("Icon Sets", "Jeux d'icônes"), list(&[Cmd::IconSets])),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::NewRule, app),
        MenuEntry::submenu(
            t("Clear Rules", "Effacer les règles"),
            list(&[Cmd::ClearSelected, Cmd::ClearSheet, Cmd::ClearTable, Cmd::ClearPivot]),
        ),
        MenuEntry::command(Cmd::Manage, app),
    ]
}

/// The cell an adornment is drawn in.
// Read by `paint_adornment` once it draws something.
#[allow(dead_code)]
pub struct CellBox<'a> {
    pub row: u32,
    pub col: u32,
    /// x0, y0, x1, y1 in grid-element coordinates.
    pub rect: (f32, f32, f32, f32),
    pub value: &'a Value,
    /// The cell's style, conditional formats included.
    pub style: &'a Style,
    /// For positions (`to_window`, `bounds_px`) and the zoom.
    pub layout: &'a GridLayout,
    pub ts: &'a WindowTextSystem,
}

/// What an adornment draws and the room it takes from the text of its
/// cell.
#[derive(Default)]
pub struct AdornmentPaint {
    /// Drawn over the cell's fill, under its text (the data bar).
    pub under: Vec<Draw>,
    /// Drawn with the text (the icon).
    pub over: Vec<Draw>,
    /// Room taken from the left and the right of the text (by the icon).
    pub text_inset: (f32, f32),
}

/// Draw what a conditional format adds inside a cell: `adornment.bar`
/// (fractions of the cell's width, already mirrored and coloured by the
/// engine) and `adornment.icon`. Called by the grid for every visible cell
/// whose `Workbook::display` carries an adornment; the grid itself leaves
/// the value out when `adornment.hide_value` is set.
pub fn paint_adornment(_adornment: &Adornment, _cell: &CellBox) -> AdornmentPaint {
    AdornmentPaint::default()
}

impl Feature for CondFormats {
    fn name(&self) -> &'static str {
        "condfmt"
    }
}
