//! Sample content (`--demo`), handy for trying the app and for screenshots.

use crate::app::ExcelApp;
use crate::i18n::t;
use gpui::Context;
use opensheet_engine::{Rect, Workbook};
use opensheet_engine::ops::BorderPreset;
use opensheet_engine::style::{Border, Color, HAlign};
use std::sync::Arc;

pub fn fill(app: &mut ExcelApp, cx: &mut Context<ExcelApp>) {
    fill_workbook(&mut app.wb);
    app.refresh(cx);
}

/// Write the sample sheet into a workbook (in the workbook's language).
pub fn fill_workbook(wb: &mut Workbook) {
    let sep = wb.locale.arg_sep;
    let locale = wb.locale;
    // Formulas are typed in the workbook's language.
    let f = |canonical: &str, _: &str| locale.fn_to_local(canonical).to_string();
    let headers = [
        t("Product", "Produit"),
        t("Region", "Région"),
        t("Quantity", "Quantité"),
        t("Unit price", "Prix unitaire"),
        t("Total", "Total"),
        t("Date", "Date"),
    ];
    for (c, h) in headers.iter().enumerate() {
        let _ = wb.set_cell_text(0, 0, c as u32, h);
    }
    let rows: [(&str, &str, u32, &str, &str); 8] = [
        ("Clavier", "Nord", 12, "49.9", "2026-01-05"),
        ("Souris", "Sud", 30, "19.5", "2026-01-12"),
        ("Écran 27\"", "Est", 7, "289", "2026-02-02"),
        ("Webcam", "Ouest", 15, "59.99", "2026-02-18"),
        ("Casque", "Nord", 22, "79", "2026-03-03"),
        ("Station d'accueil", "Sud", 5, "149.5", "2026-03-21"),
        ("Câble USB-C", "Est", 60, "9.9", "2026-04-08"),
        ("Tapis de souris", "Ouest", 41, "12", "2026-04-27"),
    ];
    let dec = wb.locale.decimal_sep.to_string();
    for (i, (p, r, q, price, date)) in rows.iter().enumerate() {
        let row = i as u32 + 1;
        let _ = wb.set_cell_text(0, row, 0, p);
        let _ = wb.set_cell_text(0, row, 1, r);
        let _ = wb.set_cell_text(0, row, 2, &q.to_string());
        let _ = wb.set_cell_text(0, row, 3, &price.replace('.', &dec));
        let _ = wb.set_cell_text(0, row, 4, &format!("=C{}*D{}", row + 1, row + 1));
        let _ = wb.set_cell_text(0, row, 5, date);
    }
    let total_row = rows.len() as u32 + 1;
    let _ = wb.set_cell_text(0, total_row, 0, t("Total", "Total"));
    let _ = wb.set_cell_text(0, total_row, 2, &format!("={}(C2:C{})", f("SUM", "SOMME"), total_row));
    let _ = wb.set_cell_text(0, total_row, 4, &format!("={}(E2:E{})", f("SUM", "SOMME"), total_row));
    let _ = wb.set_cell_text(0, total_row + 2, 0, t("Average basket", "Panier moyen"));
    let _ = wb.set_cell_text(
        0,
        total_row + 2,
        4,
        &format!("={}(E2:E{})", f("AVERAGE", "MOYENNE"), total_row),
    );
    let _ = wb.set_cell_text(0, total_row + 3, 0, t("Best seller", "Meilleure vente"));
    let _ = wb.set_cell_text(
        0,
        total_row + 3,
        4,
        &format!(
            "=INDEX(A2:A{n}{s}{m}({x}(E2:E{n}){s}E2:E{n}{s}0))",
            n = total_row,
            s = sep,
            m = f("MATCH", "EQUIV"),
            x = f("MAX", "MAX")
        ),
    );
    let _ = wb.set_cell_text(0, total_row + 4, 0, t("Orders over 500", "Commandes > 500"));
    let _ = wb.set_cell_text(
        0,
        total_row + 4,
        4,
        &format!("={}(E2:E{}{}\">500\")", f("COUNTIF", "NB.SI"), total_row, sep),
    );

    // Formatting.
    let header = Rect::new(0, 0, 0, 5);
    wb.update_style(0, header, &|s| {
        s.font.bold = true;
        s.font.color = Some(Color(0xFFFFFF));
        s.fill = Some(Color(0x107C41));
        s.h_align = HAlign::Center;
    });
    let currency: Arc<str> = Arc::from(wb.locale.currency_format);
    let cur = currency.clone();
    wb.update_style(0, Rect::new(1, 3, total_row + 2, 4), &move |s| s.num_fmt = cur.clone());
    let short: Arc<str> = Arc::from(wb.locale.short_date_format);
    wb.update_style(0, Rect::new(1, 5, total_row - 1, 5), &move |s| s.num_fmt = short.clone());
    wb.update_style(0, Rect::new(total_row, 0, total_row, 5), &|s| {
        s.font.bold = true;
        s.fill = Some(Color(0xE2F0E7));
    });
    wb.apply_borders(0, Rect::new(0, 0, total_row, 5), BorderPreset::All, Border::thin());
    for r in (2..total_row).step_by(2) {
        wb.update_style(0, Rect::new(r, 0, r, 5), &|s| s.fill = Some(Color(0xF3F9F5)));
    }
    wb.set_col_width(0, 0, Some(140.0));
    wb.set_col_width(0, 3, Some(96.0));
    wb.set_col_width(0, 4, Some(96.0));
    wb.set_col_width(0, 5, Some(88.0));
    // LOCAL TEST DATA (not committed): notes and comments to look at.
    {
        use opensheet_engine::comment::{Person, now_utc};
        let now = now_utc();
        wb.set_note(0, 1, 0, "Ada Lovelace", "Ada Lovelace:\nPrix à revoir avec le fournisseur avant la fin du mois.");
        wb.set_note(0, 3, 3, "Thomas", "Thomas:\nTarif 2026");
        wb.set_note_visible(0, 3, 3, true);
        wb.set_note(0, 6, 5, "Bob", "Bob:\nDate de la commande, pas de la livraison.");
        wb.add_comment(0, 2, 2, &Person::new("Ada Lovelace"), now - 0.2, "Quantité confirmée ?");
        wb.add_reply(0, 2, 2, &Person::new("Bob Martin"), now - 0.1, "Oui, livraison prévue lundi.\nJe mets à jour le tableau.");
        wb.add_comment(0, 5, 4, &Person::new("Chloé Dupont"), now - 1.5, "Total à vérifier");
        wb.set_comment_resolved(0, 5, 4, true);
        wb.add_comment(0, 12, 4, &Person::new("Thomas"), now - 3.0, "Formule à expliquer dans la doc.");
    }
    wb.commit("Demo");
    wb.clear_history();
    wb.modified = false;
}
