//! Flash Fill: the column of the active cell is completed after the
//! examples typed in it (the inference is the engine's `flashfill.rs`).

use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::Alert;

/// Data ▸ Flash Fill, Home ▸ Fill ▸ Flash Fill and Ctrl+E.
pub(super) fn flash_fill(app: &mut ExcelApp, cx: &mut Cx) {
    let (row, col) = app.active();
    let fill = app.wb.flash_fill_plan(app.sheet, row, col).ok();
    let Some((fill, extent)) = fill.and_then(|fill| fill.extent().map(|extent| (fill, extent))) else {
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "We looked at all the data next to your selection and didn't see a pattern for filling in values for you.\n\nTo use Flash Fill, enter a couple of examples of the output you'd like to see, keep the active cell in the column you want filled in, and click the Flash Fill button again.",
                    "Désolé... Nous avons consulté toutes les données en regard de votre sélection, mais nous n’avons pas détecté de modèle permettant de renseigner des valeurs à votre place.\n\nPour utiliser la fonctionnalité de remplissage instantané, entrez quelques exemples du résultat souhaité, gardez la cellule active dans la colonne à renseigner, puis cliquez de nouveau sur le bouton Remplissage instantané.",
                ),
            ),
            cx,
        );
        return;
    };
    if !app.may(Intent::EditCells(extent), cx) {
        return;
    }
    let sheet = app.sheet;
    app.wb.flash_fill_apply(sheet, &fill);
    app.commit_cells(t("Flash Fill", "Remplissage instantané"), &[extent], ChangeCause::Fill, cx);
    // What Excel says on the status bar once the column is filled.
    let done = t("Flash Fill Changed Cells", "Compléter les cellules modifiées à l’aide du remplissage instantané");
    app.set_status(format!("{done}: {}", fill.cells.len()), cx);
}
