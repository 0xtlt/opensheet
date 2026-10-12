//! Formatting charts: the commands of the "Chart Design" tab that restyle
//! a chart, the "Format" tab, and the Format pane (Format Chart Area,
//! Format Data Series, ...).
//!
//! Not written yet: the commands are in place and say so; the "Format"
//! tab appears while a chart is selected. See `features/mod.rs` (the pane
//! is a view opened with `open_pane` and built with `widgets`; the
//! selection and the placement of charts are `charts.rs`).

use super::{Command, ContextMenu, ContextTab, Cx, Feature, MenuSection, MenuTarget, charts};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, rows};
use gpui::{AnyElement, Window, div, prelude::*};

pub struct ChartFormat;

const FORMAT_TAB: &str = "chart-format";

commands! {
    /// What the user can ask of chart formatting.
    pub enum Cmd {
        // Chart Design
        QuickLayout = "chart-bar", t("Quick Layout", "Disposition rapide");
        ChangeColors = "fill", t("Change Colors", "Modifier les couleurs");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
        SelectData = "table", t("Select Data", "Sélectionner des données");
        MoveChart = "chart-bar", t("Move Chart", "Déplacer le graphique");
        // Format
        Element = "", t("Chart Elements", "Éléments de graphique");
        FormatSelection = "format-cells", t("Format Selection", "Mise en forme de la sélection");
        ResetStyle = "refresh", t("Reset to Match Style", "Rétablir le style d'origine");
        ShapeFill = "fill", t("Shape Fill", "Remplissage");
        ShapeOutline = "borders", t("Shape Outline", "Contour");
        ShapeEffects = "shapes", t("Shape Effects", "Effets");
        TextFill = "fill", t("Text Fill", "Remplissage du texte");
        TextOutline = "borders", t("Text Outline", "Contour du texte");
        TextEffects = "wordart", t("Text Effects", "Effets du texte");
        // Context menu
        FormatPane = "format-cells", t("Format Chart Area…", "Format de la zone de graphique…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
        app.not_implemented(self.label(), cx)
    }
}

/// The groups of the "Format" tab of charts.
fn format_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Current Selection", "Sélection active"),
            rows(vec![
                app.combo(
                    "cf-element",
                    t("Chart Area", "Zone de graphique"),
                    150.0,
                    Cmd::Element.label(),
                    cx,
                    |this, window, cx| Cmd::Element.run(this, window, cx),
                ),
                app.cmd_labeled("cf-format", Cmd::FormatSelection, false, cx),
                app.cmd_labeled("cf-reset", Cmd::ResetStyle, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Shape Styles", "Styles de formes"),
            rows(vec![
                app.cmd_labeled("cf-fill", Cmd::ShapeFill, true, cx),
                app.cmd_labeled("cf-outline", Cmd::ShapeOutline, true, cx),
                app.cmd_labeled("cf-effects", Cmd::ShapeEffects, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("WordArt Styles", "Styles WordArt"),
            rows(vec![
                app.cmd_labeled("cf-text-fill", Cmd::TextFill, true, cx),
                app.cmd_labeled("cf-text-outline", Cmd::TextOutline, true, cx),
                app.cmd_labeled("cf-text-effects", Cmd::TextEffects, true, cx),
            ]),
        )
        .into_any_element(),
        div().h_full().child(app.arrange_group("ar", cx).border_r_0()).into_any_element(),
    ]
}

impl Feature for ChartFormat {
    fn name(&self) -> &'static str {
        "chart_format"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match app.selected_chart() {
            Some(_) => vec![ContextTab {
                id: FORMAT_TAB,
                title: t_ctx("tab", "Format", "Mise en forme"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        format_tab(app, cx)
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Object(id) if id.owner == super::index_of(&charts::Charts)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::FormatPane, app));
        }
    }
}
