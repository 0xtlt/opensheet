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
    cf_test(wb);
    wb.commit("Demo");
    wb.clear_history();
    wb.modified = false;
}

// TEST AID (not committed): every drawing of conditional formats at once.
fn cf_test(wb: &mut Workbook) {
    use opensheet_engine::condfmt::*;
    let mut rules: Vec<CondFormat> = Vec::new();
    let col = |c: u32| Rect::new(1, c, 8, c);
    // H..L: bars
    let neg = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0];
    for c in 7..12u32 {
        for (i, v) in neg.iter().enumerate() {
            let v = if c >= 9 { v + 4.0 } else { *v };
            let _ = wb.set_cell_text(0, 1 + i as u32, c, &format!("{}", v as i64));
        }
    }
    rules.push(CondFormat::new(vec![col(7)], Rule::DataBar(DataBar::preset(BarColor::Blue, true))));
    let mut mid = DataBar::preset(BarColor::Green, false);
    mid.axis = BarAxis::Midpoint;
    rules.push(CondFormat::new(vec![col(8)], Rule::DataBar(mid)));
    let mut rtl = DataBar::preset(BarColor::Orange, false);
    rtl.direction = BarDirection::RightToLeft;
    rules.push(CondFormat::new(vec![col(9)], Rule::DataBar(rtl)));
    let mut only = DataBar::preset(BarColor::Purple, true);
    only.show_value = false;
    rules.push(CondFormat::new(vec![col(10)], Rule::DataBar(only)));
    let mut bordered = DataBar::preset(BarColor::Red, false);
    bordered.border = Some(opensheet_engine::Color(0x000000));
    rules.push(CondFormat::new(vec![col(11)], Rule::DataBar(bordered)));
    rules.push(CondFormat::new(vec![Rect::new(1, 4, 8, 4)], Rule::ColorScale(ScalePreset::GreenYellowRed.scale())));
    // rows 12..: one icon set per row, values 1..5 in H..L; M: icon only, centred
    for (i, set) in IconSet::ALL.iter().enumerate() {
        let row = 11 + i as u32;
        for k in 0..5u32 {
            let _ = wb.set_cell_text(0, row, 7 + k, &format!("{}", k + 1));
            let _ = wb.set_cell_text(0, row, 13 + k, &format!("{}", k + 1));
        }
        rules.push(CondFormat::new(vec![Rect::new(row, 7, row, 11)], Rule::IconSet(IconRule::preset(*set))));
        let mut alone = IconRule::preset(*set);
        alone.show_value = false;
        rules.push(CondFormat::new(vec![Rect::new(row, 13, row, 17)], Rule::IconSet(alone)));
    }
    wb.update_style(0, Rect::new(11, 13, 40, 17), &|s| s.h_align = HAlign::Center);
    for c in 13..18 {
        wb.set_col_width(0, c, Some(28.0));
    }
    wb.set_cond_formats(0, rules);
}
