//! The names of the built-in themes, colour sets, font pairs and effects
//! in the language of the interface, as Excel shows them (from its own
//! resources). Names that are the same in every language, most of them
//! names of fonts, are not listed.

use crate::i18n::t;

/// `name` (the English name files carry) as the galleries and menus show
/// it. A name that is not Excel's — a theme of the user's — is shown as
/// it is.
pub fn shown(name: &str) -> String {
    let known = match name {
        "Office 2013 - 2022" => t("Office 2013 - 2022", "Office 2013 - 2022"),
        "Facet" => t("Facet", "Facette"),
        "Integral" => t("Integral", "Intégral"),
        "Ion" => t("Ion", "Ion"),
        "Ion Boardroom" => t("Ion Boardroom", "Salle d’ions"),
        "Organic" => t("Organic", "Organique"),
        "Retrospect" => t("Retrospect", "Rétrospective"),
        "Slice" => t("Slice", "Secteur"),
        "Wisp" => t("Wisp", "Brin"),
        "Atlas" => t("Atlas", "Atlas"),
        "Badge" => t("Badge", "Badge"),
        "Berlin" => t("Berlin", "Berlin"),
        "Celestial" => t("Celestial", "Céleste"),
        "Crop" => t("Crop", "Cadrage"),
        "Depth" => t("Depth", "Profondeur"),
        "Droplet" => t("Droplet", "Ronds dans l’eau"),
        "Feathered" => t("Feathered", "À plumes"),
        "Gallery" => t("Gallery", "Galerie"),
        "Headlines" => t("Headlines", "Titres"),
        "Main Event" => t("Main Event", "Grand événement"),
        "Mesh" => t("Mesh", "Maillage"),
        "Parallax" => t("Parallax", "Parallaxe"),
        "Parcel" => t("Parcel", "Colis"),
        "Savon" => t("Savon", "Savon"),
        "Vapor Trail" => t("Vapor Trail", "Traînée de condensation"),
        "View" => t("View", "Vue"),
        "Wood Type" => t("Wood Type", "Type de bois"),
        "Grayscale" => t("Grayscale", "Nuances de gris"),
        "Blue Warm" => t("Blue Warm", "Bleu chaud"),
        "Blue" => t("Blue", "Bleu"),
        "Blue II" => t("Blue II", "Bleu II"),
        "Blue Green" => t("Blue Green", "Bleu vert"),
        "Green" => t("Green", "Vert"),
        "Green Yellow" => t("Green Yellow", "Vert jaune"),
        "Yellow" => t("Yellow", "Jaune"),
        "Yellow Orange" => t("Yellow Orange", "Jaune orange"),
        "Orange" => t("Orange", "Orange"),
        "Orange Red" => t("Orange Red", "Orange rouge"),
        "Red Orange" => t("Red Orange", "Rouge orange"),
        "Red" => t("Red", "Rouge"),
        "Red Violet" => t("Red Violet", "Rouge violet"),
        "Violet" => t("Violet", "Violet"),
        "Violet II" => t("Violet II", "Violet II"),
        "Median" => t("Median", "Médian"),
        "Paper" => t("Paper", "Papier"),
        "Marquee" => t("Marquee", "Palissade"),
        "Slipstream" => t("Slipstream", "Sillage"),
        "Aspect" => t("Aspect", "Aspect"),
        "Tw Cen MT" => t("Tw Cen MT", "Tw Cen MT"),
        "Subtle Solids" => t("Subtle Solids", "Solides discrets"),
        "Banded Edge" => t("Banded Edge", "Coin à bandes"),
        "Smokey Glass" => t("Smokey Glass", "Verre de fumée"),
        "Glow Edge" => t("Glow Edge", "Coin lumineux"),
        "Grunge Texture" => t("Grunge Texture", "Texture grunge"),
        "Frosted Glass" => t("Frosted Glass", "Verre givré"),
        "Top Shadow" => t("Top Shadow", "Ombre supérieure"),
        "Inset" => t("Inset", "Incrustation"),
        "Milk Glass" => t("Milk Glass", "Verre de lait"),
        "Riblet" => t("Riblet", "Riblet"),
        "Reflection" => t("Reflection", "Reflet"),
        "Extreme Shadow" => t("Extreme Shadow", "Ombre extrême"),
        "Glossy" => t("Glossy", "Glacé"),
        _ => return name.to_string(),
    };
    known.to_string()
}
