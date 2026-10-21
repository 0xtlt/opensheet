//! Paper sizes: the list of the Page Setup dialog, with dimensions.

use crate::locale::Locale;

/// Millimetres per inch.
const MM: f64 = 25.4;

/// A paper size, identified by its OOXML `paperSize` code (the numbers of
/// the Windows printing API, which is also what Excel stores).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaperSize(pub u16);

/// One entry of the paper list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaperInfo {
    pub size: PaperSize,
    /// English name ("Letter", "Envelope #10").
    pub name: &'static str,
    /// Width and height in portrait orientation, in millimetres.
    pub mm: (f64, f64),
    /// Whether the size is defined in inches (decides how it is spelled:
    /// `8.5" x 11"` or `21 cm x 29.7 cm`).
    pub imperial: bool,
}

const fn inch(code: u16, name: &'static str, w: f64, h: f64) -> PaperInfo {
    PaperInfo {
        size: PaperSize(code),
        name,
        mm: (w * MM, h * MM),
        imperial: true,
    }
}

const fn metric(code: u16, name: &'static str, w: f64, h: f64) -> PaperInfo {
    PaperInfo {
        size: PaperSize(code),
        name,
        mm: (w, h),
        imperial: false,
    }
}

/// Every size Excel can store, in code order.
static PAPERS: [PaperInfo; 66] = [
    inch(1, "Letter", 8.5, 11.0),
    inch(2, "Letter Small", 8.5, 11.0),
    inch(3, "Tabloid", 11.0, 17.0),
    inch(4, "Ledger", 17.0, 11.0),
    inch(5, "Legal", 8.5, 14.0),
    inch(6, "Statement", 5.5, 8.5),
    inch(7, "Executive", 7.25, 10.5),
    metric(8, "A3", 297.0, 420.0),
    metric(9, "A4", 210.0, 297.0),
    metric(10, "A4 Small", 210.0, 297.0),
    metric(11, "A5", 148.0, 210.0),
    metric(12, "B4 (JIS)", 257.0, 364.0),
    metric(13, "B5 (JIS)", 182.0, 257.0),
    inch(14, "Folio", 8.5, 13.0),
    metric(15, "Quarto", 215.0, 275.0),
    inch(16, "10x14", 10.0, 14.0),
    inch(17, "11x17", 11.0, 17.0),
    inch(18, "Note", 8.5, 11.0),
    inch(19, "Envelope #9", 3.875, 8.875),
    inch(20, "Envelope #10", 4.125, 9.5),
    inch(21, "Envelope #11", 4.5, 10.375),
    inch(22, "Envelope #12", 4.75, 11.0),
    inch(23, "Envelope #14", 5.0, 11.5),
    inch(24, "C size sheet", 17.0, 22.0),
    inch(25, "D size sheet", 22.0, 34.0),
    inch(26, "E size sheet", 34.0, 44.0),
    metric(27, "Envelope DL", 110.0, 220.0),
    metric(28, "Envelope C5", 162.0, 229.0),
    metric(29, "Envelope C3", 324.0, 458.0),
    metric(30, "Envelope C4", 229.0, 324.0),
    metric(31, "Envelope C6", 114.0, 162.0),
    metric(32, "Envelope C65", 114.0, 229.0),
    metric(33, "Envelope B4", 250.0, 353.0),
    metric(34, "Envelope B5", 176.0, 250.0),
    metric(35, "Envelope B6", 176.0, 125.0),
    metric(36, "Envelope Italy", 110.0, 230.0),
    inch(37, "Envelope Monarch", 3.875, 7.5),
    inch(38, "6 3/4 Envelope", 3.625, 6.5),
    inch(39, "US Std Fanfold", 14.875, 11.0),
    inch(40, "German Std Fanfold", 8.5, 12.0),
    inch(41, "German Legal Fanfold", 8.5, 13.0),
    metric(42, "B4 (ISO)", 250.0, 353.0),
    metric(43, "Japanese Postcard", 100.0, 148.0),
    inch(44, "9x11", 9.0, 11.0),
    inch(45, "10x11", 10.0, 11.0),
    inch(46, "15x11", 15.0, 11.0),
    metric(47, "Envelope Invite", 220.0, 220.0),
    inch(50, "Letter Extra", 9.275, 12.0),
    inch(51, "Legal Extra", 9.275, 15.0),
    inch(52, "Tabloid Extra", 11.69, 18.0),
    metric(53, "A4 Extra", 236.0, 322.0),
    inch(54, "Letter Transverse", 8.275, 11.0),
    metric(55, "A4 Transverse", 210.0, 297.0),
    inch(56, "Letter Extra Transverse", 9.275, 12.0),
    metric(57, "Super A", 227.0, 356.0),
    metric(58, "Super B", 305.0, 487.0),
    inch(59, "Letter Plus", 8.5, 12.69),
    metric(60, "A4 Plus", 210.0, 330.0),
    metric(61, "A5 Transverse", 148.0, 210.0),
    metric(62, "B5 (JIS) Transverse", 182.0, 257.0),
    metric(63, "A3 Extra", 322.0, 445.0),
    metric(64, "A5 Extra", 174.0, 235.0),
    metric(65, "B5 (ISO) Extra", 201.0, 276.0),
    metric(66, "A2", 420.0, 594.0),
    metric(67, "A3 Transverse", 297.0, 420.0),
    metric(68, "A3 Extra Transverse", 322.0, 445.0),
];

/// Names that are translated, in en, fr, es, de, it order. The other sizes
/// are spelled the same in every language, apart from the word "Envelope".
const TRANSLATED: [[&str; 5]; 6] = [
    ["Letter", "Lettre", "Carta", "Letter", "Lettera"],
    ["Tabloid", "Tabloïd", "Tabloide", "Tabloid", "Tabloid"],
    ["Legal", "Légal", "Oficio", "Legal", "Legale"],
    ["Statement", "Relevé", "Estamento", "Statement", "Statement"],
    [
        "Executive",
        "Exécutive",
        "Ejecutivo",
        "Executive",
        "Executive",
    ],
    ["Envelope", "Enveloppe", "Sobre", "Umschlag", "Busta"],
];

fn locale_index(locale: &Locale) -> usize {
    match locale.id {
        "fr" => 1,
        "es" => 2,
        "de" => 3,
        "it" => 4,
        _ => 0,
    }
}

impl PaperSize {
    pub const LETTER: PaperSize = PaperSize(1);
    pub const TABLOID: PaperSize = PaperSize(3);
    pub const LEGAL: PaperSize = PaperSize(5);
    pub const STATEMENT: PaperSize = PaperSize(6);
    pub const EXECUTIVE: PaperSize = PaperSize(7);
    pub const A3: PaperSize = PaperSize(8);
    pub const A4: PaperSize = PaperSize(9);
    pub const A5: PaperSize = PaperSize(11);
    pub const B4_JIS: PaperSize = PaperSize(12);
    pub const B5_JIS: PaperSize = PaperSize(13);
    pub const ENVELOPE_10: PaperSize = PaperSize(20);
    pub const ENVELOPE_DL: PaperSize = PaperSize(27);
    pub const ENVELOPE_C5: PaperSize = PaperSize(28);
    pub const ENVELOPE_MONARCH: PaperSize = PaperSize(37);

    /// The sizes of the ribbon's Size gallery, in Excel's order.
    pub const COMMON: [PaperSize; 14] = [
        PaperSize::LETTER,
        PaperSize::TABLOID,
        PaperSize::LEGAL,
        PaperSize::STATEMENT,
        PaperSize::EXECUTIVE,
        PaperSize::A3,
        PaperSize::A4,
        PaperSize::A5,
        PaperSize::B4_JIS,
        PaperSize::B5_JIS,
        PaperSize::ENVELOPE_10,
        PaperSize::ENVELOPE_DL,
        PaperSize::ENVELOPE_C5,
        PaperSize::ENVELOPE_MONARCH,
    ];

    /// Every known size, in code order (the Page Setup dialog's list).
    pub fn all() -> &'static [PaperInfo] {
        &PAPERS
    }

    /// The paper of a new sheet: Letter where Excel is in English, A4
    /// elsewhere.
    pub fn default_for(locale: &Locale) -> PaperSize {
        if locale.id == "en" {
            PaperSize::LETTER
        } else {
            PaperSize::A4
        }
    }

    /// `None` for a code that is not in Excel's list (a printer-specific
    /// size).
    pub fn info(self) -> Option<&'static PaperInfo> {
        PAPERS.iter().find(|p| p.size == self)
    }

    /// Width and height in portrait orientation, in millimetres. A size
    /// this list does not know prints on Letter, like a missing printer
    /// form does in Excel.
    pub fn size_mm(self) -> (f64, f64) {
        self.info().map_or(PAPERS[0].mm, |p| p.mm)
    }

    /// Width and height in portrait orientation, in points (1/72 inch).
    pub fn size_pt(self) -> (f64, f64) {
        let (w, h) = self.size_mm();
        (w / MM * 72.0, h / MM * 72.0)
    }

    /// The name shown in the Size gallery ("Letter", "Lettre", "A4").
    pub fn name(self, locale: &Locale) -> String {
        let Some(info) = self.info() else {
            return format!("#{}", self.0);
        };
        let li = locale_index(locale);
        let mut name = info.name.to_string();
        for row in &TRANSLATED {
            if name.contains(row[0]) && row[0] != row[li] {
                // Translated envelopes put the word first ("Enveloppe 6 3/4").
                if row[0] == "Envelope" && name.ends_with(" Envelope") {
                    name = format!("{} {}", row[li], name.trim_end_matches(" Envelope"));
                } else {
                    name = name.replacen(row[0], row[li], 1);
                }
                break;
            }
        }
        name
    }

    /// The dimensions as the Size gallery spells them: inches in English
    /// (`8.5" x 11"`), centimetres elsewhere (`21 cm x 29,7 cm`).
    pub fn dimensions_text(self, locale: &Locale) -> String {
        let (w, h) = self.size_mm();
        let number = |v: f64| -> String {
            let s = format!("{:.2}", v);
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.replace('.', &locale.decimal_sep.to_string())
        };
        if locale.id == "en" {
            format!("{}\" x {}\"", number(w / MM), number(h / MM))
        } else {
            format!("{} cm x {} cm", number(w / 10.0), number(h / 10.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_names() {
        assert_eq!(PaperSize::default_for(Locale::en()), PaperSize::LETTER);
        assert_eq!(PaperSize::default_for(Locale::fr()), PaperSize::A4);
        assert_eq!(PaperSize::LETTER.size_pt(), (612.0, 792.0));
        let (w, h) = PaperSize::A4.size_pt();
        assert!((w - 595.28).abs() < 0.01 && (h - 841.89).abs() < 0.01);
        assert_eq!(PaperSize::LETTER.name(Locale::fr()), "Lettre");
        assert_eq!(PaperSize::ENVELOPE_10.name(Locale::fr()), "Enveloppe #10");
        assert_eq!(PaperSize(38).name(Locale::de()), "Umschlag 6 3/4");
        assert_eq!(PaperSize::A4.name(Locale::es()), "A4");
        assert_eq!(
            PaperSize::LETTER.dimensions_text(Locale::en()),
            "8.5\" x 11\""
        );
        assert_eq!(
            PaperSize::A4.dimensions_text(Locale::en()),
            "8.27\" x 11.69\""
        );
        assert_eq!(
            PaperSize::A4.dimensions_text(Locale::fr()),
            "21 cm x 29,7 cm"
        );
        assert_eq!(
            PaperSize::LETTER.dimensions_text(Locale::fr()),
            "21,59 cm x 27,94 cm"
        );
        // An unknown printer form falls back to Letter.
        assert_eq!(PaperSize(256).size_pt(), (612.0, 792.0));
        // Codes are unique and sorted.
        assert!(
            PaperSize::all()
                .windows(2)
                .all(|w| w[0].size.0 < w[1].size.0)
        );
        for p in PaperSize::COMMON {
            assert!(p.info().is_some());
        }
    }
}
