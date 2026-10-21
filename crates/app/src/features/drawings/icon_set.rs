//! The pictograms of Insert ▸ Icons.
//!
//! Excel's own gallery is stock content served online, which cannot be
//! given away with another application; and no openly licensed set comes
//! with this project's dependencies. These icons were drawn for
//! OpenSheet: solid shapes on a square of 24 units, each an SVG document
//! of its own once inserted, so that it scales, takes a Graphics Fill
//! and travels in a file as any vector picture does.

use crate::i18n::t;

/// One pictogram.
pub struct Icon {
    /// Index in [`categories`].
    pub category: usize,
    /// What it is called: its tooltip, what Search looks for, and the
    /// description it is inserted with.
    pub name: &'static str,
    /// The same in English, which Search also understands.
    pub english: &'static str,
    /// The shapes, inside a view box of 24 × 24.
    body: &'static str,
}

impl Icon {
    /// The SVG document of the icon: one inch wide when inserted.
    pub fn svg(&self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 24 24">{}</svg>"#,
            self.body
        )
    }
}

/// The categories of the dialog, in the order of its list.
pub fn categories() -> Vec<&'static str> {
    vec![
        t("People", "Personnes"),
        t("Technology and electronics", "Technologie et électronique"),
        t("Communication", "Communication"),
        t("Business", "Entreprise"),
        t("Commerce", "Commerce"),
        t("Nature and outdoors", "Nature et plein air"),
        t("Weather and seasons", "Météo et saisons"),
        t("Places", "Lieux"),
        t("Signs and symbols", "Signes et symboles"),
        t("Arrows", "Flèches"),
        t("Education", "Éducation"),
        t("Arts", "Arts"),
        t("Food and drinks", "Aliments et boissons"),
    ]
}

/// Every icon, category after category.
pub fn icons() -> Vec<Icon> {
    let icon = |category, name, english, body| Icon {
        category,
        name,
        english,
        body,
    };
    vec![
        // person
        icon(
            0,
            t("Person", "Personne"),
            "Person",
            r##"<circle cx="12" cy="7" r="4"/><path d="M4 21c0-4.4 3.6-8 8-8s8 3.6 8 8z"/>"##,
        ),
        // people
        icon(
            0,
            t("Group of people", "Groupe de personnes"),
            "Group of people",
            r##"<circle cx="8" cy="8" r="3.2"/><circle cx="17" cy="9" r="2.6"/><path d="M1.5 20c0-3.6 2.9-6.5 6.5-6.5s6.5 2.9 6.5 6.5z"/><path d="M16 20c0-2-.7-3.9-1.9-5.3.9-.4 1.9-.7 2.9-.7 3 0 5.5 2.5 5.5 6z"/>"##,
        ),
        // heart
        icon(
            0,
            t("Heart", "Cœur"),
            "Heart",
            r##"<path d="M12 21C5 15.5 2 12.2 2 8.5 2 5.5 4.4 3 7.3 3c1.8 0 3.6.9 4.7 2.4C13.1 3.9 14.9 3 16.7 3 19.6 3 22 5.5 22 8.5c0 3.7-3 7-10 12.5z"/>"##,
        ),
        // smiley
        icon(
            0,
            t("Smiling face", "Visage souriant"),
            "Smiling face",
            r##"<path fill-rule="evenodd" d="M12 2a10 10 0 100 20 10 10 0 000-20zM8.5 8a1.5 1.5 0 110 3 1.5 1.5 0 010-3zm7 0a1.5 1.5 0 110 3 1.5 1.5 0 010-3zM7 13.5h10a5 5 0 01-10 0z"/>"##,
        ),
        // laptop
        icon(
            1,
            t("Laptop", "Ordinateur portable"),
            "Laptop",
            r##"<path fill-rule="evenodd" d="M4 5h16v11H4zm2 2v7h12V7z"/><path d="M1 17.5h22V19a1 1 0 01-1 1H2a1 1 0 01-1-1z"/>"##,
        ),
        // smartphone
        icon(
            1,
            t("Smartphone", "Smartphone"),
            "Smartphone",
            r##"<path fill-rule="evenodd" d="M7 2h10a1 1 0 011 1v18a1 1 0 01-1 1H7a1 1 0 01-1-1V3a1 1 0 011-1zm1 3v13h8V5zm4 14.2a.9.9 0 100 1.8.9.9 0 000-1.8z"/>"##,
        ),
        // monitor
        icon(
            1,
            t("Monitor", "Écran"),
            "Monitor",
            r##"<path fill-rule="evenodd" d="M2 4h20v13H2zm2 2v9h16V6z"/><path d="M9 18h6v2h3v1.5H6V20h3z"/>"##,
        ),
        // printer
        icon(
            1,
            t("Printer", "Imprimante"),
            "Printer",
            r##"<path d="M7 3h10v4H7z"/><path fill-rule="evenodd" d="M3 8h18a1 1 0 011 1v8h-4v-3H6v3H2V9a1 1 0 011-1zm15 2a1 1 0 100 2 1 1 0 000-2z"/><path d="M7 15.5h10V21H7z"/>"##,
        ),
        // camera
        icon(
            1,
            t("Camera", "Appareil photo"),
            "Camera",
            r##"<path fill-rule="evenodd" d="M9 4h6l1.5 2.5H21a1 1 0 011 1V19a1 1 0 01-1 1H3a1 1 0 01-1-1V7.5a1 1 0 011-1h4.5zm3 5a4 4 0 100 8 4 4 0 000-8zm0 2a2 2 0 110 4 2 2 0 010-4z"/>"##,
        ),
        // gear
        icon(
            1,
            t("Gear", "Engrenage"),
            "Gear",
            r##"<rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(0 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(45 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(90 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(135 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(180 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(225 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(270 12 12)"/><rect x="10.25" y="1.5" width="3.5" height="5" rx="0.6" transform="rotate(315 12 12)"/><path fill-rule="evenodd" d="M12 4.5a7.5 7.5 0 100 15 7.5 7.5 0 000-15zm0 4.5a3 3 0 110 6 3 3 0 010-6z"/>"##,
        ),
        // envelope
        icon(
            2,
            t("Envelope", "Enveloppe"),
            "Envelope",
            r##"<path d="M2 5h20v1.2l-10 6.3L2 6.2z"/><path d="M2 8.5l10 6.3 10-6.3V19H2z"/>"##,
        ),
        // chat
        icon(
            2,
            t("Speech bubble", "Bulle"),
            "Speech bubble",
            r##"<path d="M3 4h18a1 1 0 011 1v11a1 1 0 01-1 1h-9l-5 4v-4H3a1 1 0 01-1-1V5a1 1 0 011-1z"/>"##,
        ),
        // megaphone
        icon(
            2,
            t("Megaphone", "Mégaphone"),
            "Megaphone",
            r##"<path d="M3 9h4l9-5v16l-9-5H3z"/><path d="M18 8.5a4 4 0 010 7z"/><path d="M5 16.5h3l1 4.5H6z"/>"##,
        ),
        // bell
        icon(
            2,
            t("Bell", "Cloche"),
            "Bell",
            r##"<path d="M12 2a2 2 0 012 2v.6A6 6 0 0118 10v5l2 3H4l2-3v-5a6 6 0 014-5.4V4a2 2 0 012-2z"/><path d="M9.5 19.5h5a2.5 2.5 0 01-5 0z"/>"##,
        ),
        // briefcase
        icon(
            3,
            t("Briefcase", "Porte-documents"),
            "Briefcase",
            r##"<path fill-rule="evenodd" d="M9 3h6a1 1 0 011 1v3h5a1 1 0 011 1v4H2V8a1 1 0 011-1h5V4a1 1 0 011-1zm1 2v2h4V5z"/><path d="M2 13.5h8V15h4v-1.5h8V20a1 1 0 01-1 1H3a1 1 0 01-1-1z"/>"##,
        ),
        // bar-chart
        icon(
            3,
            t("Bar chart", "Graphique à barres"),
            "Bar chart",
            r##"<path d="M3 13h4v8H3zM10 8h4v13h-4zM17 3h4v18h-4z"/>"##,
        ),
        // pie-chart
        icon(
            3,
            t("Pie chart", "Graphique en secteurs"),
            "Pie chart",
            r##"<path d="M11 4v9h9a9 9 0 11-9-9z"/><path d="M13 2a9 9 0 019 9h-9z"/>"##,
        ),
        // trend
        icon(
            3,
            t("Upward trend", "Tendance à la hausse"),
            "Upward trend",
            r##"<path d="M3 17.5l6-6 4 4 5.7-5.7V13H21V6h-7v2.3h3.2L13 12.5l-4-4-7.5 7.5z"/>"##,
        ),
        // target
        icon(
            3,
            t("Target", "Cible"),
            "Target",
            r##"<path fill-rule="evenodd" d="M12 2a10 10 0 100 20 10 10 0 000-20zm0 3a7 7 0 110 14 7 7 0 010-14zm0 3a4 4 0 100 8 4 4 0 000-8zm0 2.5a1.5 1.5 0 110 3 1.5 1.5 0 010-3z"/>"##,
        ),
        // trophy
        icon(
            3,
            t("Trophy", "Trophée"),
            "Trophy",
            r##"<path fill-rule="evenodd" d="M7 3h10v2h4v3a4 4 0 01-4.3 4A5 5 0 0113 14.9V18h3v3H8v-3h3v-3.1A5 5 0 017.3 12 4 4 0 013 8V5h4zm0 4H5v1a2 2 0 002 2zm10 0v3a2 2 0 002-2V7z"/>"##,
        ),
        // lightbulb
        icon(
            3,
            t("Light bulb", "Ampoule"),
            "Light bulb",
            r##"<path d="M12 2a7 7 0 014 12.7V17H8v-2.3A7 7 0 0112 2z"/><path d="M8.5 18.5h7V20h-7zM10 21h4v1h-4z"/>"##,
        ),
        // calendar
        icon(
            3,
            t("Calendar", "Calendrier"),
            "Calendar",
            r##"<path fill-rule="evenodd" d="M7 2h2v2h6V2h2v2h3a1 1 0 011 1v15a1 1 0 01-1 1H4a1 1 0 01-1-1V5a1 1 0 011-1h3zM5 9v10h14V9zm2 2h3v3H7z"/>"##,
        ),
        // clock
        icon(
            3,
            t("Clock", "Horloge"),
            "Clock",
            r##"<path fill-rule="evenodd" d="M12 2a10 10 0 100 20 10 10 0 000-20zm-1 4h2v5.6l3.7 2.1-1 1.7L11 12.7z"/>"##,
        ),
        // banknote
        icon(
            3,
            t("Banknote", "Billet de banque"),
            "Banknote",
            r##"<path fill-rule="evenodd" d="M2 6h20v12H2zm10 2.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7zM4 8v2a2 2 0 002-2zm16 0h-2a2 2 0 002 2zM4 16h2a2 2 0 00-2-2zm16 0v-2a2 2 0 00-2 2z"/>"##,
        ),
        // cart
        icon(
            4,
            t("Shopping cart", "Panier"),
            "Shopping cart",
            r##"<path d="M2 3h3.5l.7 3H22l-2.5 8H8.3l.4 2H19v2H7L4 5H2z"/><circle cx="9" cy="20.2" r="1.8"/><circle cx="17" cy="20.2" r="1.8"/>"##,
        ),
        // tag
        icon(
            4,
            t("Price tag", "Étiquette de prix"),
            "Price tag",
            r##"<path fill-rule="evenodd" d="M3 3h8l10 10-8 8L3 11zm4.5 3a1.5 1.5 0 100 3 1.5 1.5 0 000-3z"/>"##,
        ),
        // gift
        icon(
            4,
            t("Gift", "Cadeau"),
            "Gift",
            r##"<path d="M3 8h8v4H3zM13 8h8v4h-8zM4 13.5h7V21H4zM13 13.5h7V21h-7z"/><path d="M12 7C10 3 5.5 3 6 6c.2 1 1.5 1.5 6 1zm0 0c2-4 6.5-4 6-1-.2 1-1.5 1.5-6 1z"/>"##,
        ),
        // store
        icon(
            4,
            t("Store", "Magasin"),
            "Store",
            r##"<path d="M3 4h18l1 5a3 3 0 01-3.3 2.4 3 3 0 01-3.4 0 3 3 0 01-3.3.2 3 3 0 01-3.3-.2A3 3 0 012 9z"/><path fill-rule="evenodd" d="M4 13h16v8H4zm2 2v4h5v-4z"/>"##,
        ),
        // leaf
        icon(
            5,
            t("Leaf", "Feuille"),
            "Leaf",
            r##"<path d="M20 3c1 9-2 16-10 16-1.5 0-2.8-.3-3.9-.9L4 21l-1.4-1.4 2.2-2.2C3 14 4 9 8 6.5 11 4.6 15 4 20 3z"/>"##,
        ),
        // tree
        icon(
            5,
            t("Tree", "Arbre"),
            "Tree",
            r##"<path d="M12 2l6 8h-3l4 6h-5.5v5h-3v-5H5l4-6H6z"/>"##,
        ),
        // flame
        icon(
            5,
            t("Flame", "Flamme"),
            "Flame",
            r##"<path d="M12 2c1 4 6 6 6 11.5a6 6 0 01-12 0c0-2.5 1.2-4 2.5-5.5.3 1.8 1 2.8 2 3.2C10 8 10.5 5 12 2z"/>"##,
        ),
        // mountain
        icon(
            5,
            t("Mountains", "Montagnes"),
            "Mountains",
            r##"<path d="M2 20L9 6l4 7 2.5-3.5L22 20z"/>"##,
        ),
        // drop
        icon(
            5,
            t("Water drop", "Goutte d'eau"),
            "Water drop",
            r##"<path d="M12 2c4 5 6.5 8.5 6.5 12a6.5 6.5 0 01-13 0C5.5 10.5 8 7 12 2z"/>"##,
        ),
        // sun
        icon(
            6,
            t("Sun", "Soleil"),
            "Sun",
            r##"<circle cx="12" cy="12" r="4.5"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(0 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(45 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(90 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(135 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(180 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(225 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(270 12 12)"/><rect x="11.2" y="1.5" width="1.6" height="4" rx="0.8" transform="rotate(315 12 12)"/>"##,
        ),
        // moon
        icon(
            6,
            t("Moon", "Lune"),
            "Moon",
            r##"<path d="M20 14.5A8.5 8.5 0 019.5 4 8.5 8.5 0 1020 14.5z"/>"##,
        ),
        // cloud
        icon(
            6,
            t("Cloud", "Nuage"),
            "Cloud",
            r##"<path d="M7 19a5 5 0 01-.6-9.96A6 6 0 0118 9.5a4.8 4.8 0 01-.5 9.5z"/>"##,
        ),
        // lightning
        icon(
            6,
            t("Lightning", "Éclair"),
            "Lightning",
            r##"<path d="M13 2L4 14h6l-1 8 9-12h-6z"/>"##,
        ),
        // home
        icon(
            7,
            t("House", "Maison"),
            "House",
            r##"<path d="M12 3l10 9h-3v9h-5v-6h-4v6H5v-9H2z"/>"##,
        ),
        // building
        icon(
            7,
            t("Building", "Bâtiment"),
            "Building",
            r##"<path fill-rule="evenodd" d="M5 2h14v20h-5v-4h-4v4H5zm3 3v3h3V5zm5 0v3h3V5zM8 10v3h3v-3zm5 0v3h3v-3z"/>"##,
        ),
        // pin
        icon(
            7,
            t("Map pin", "Repère de carte"),
            "Map pin",
            r##"<path fill-rule="evenodd" d="M12 2a7 7 0 017 7c0 5-7 13-7 13S5 14 5 9a7 7 0 017-7zm0 4.5a2.5 2.5 0 100 5 2.5 2.5 0 000-5z"/>"##,
        ),
        // car
        icon(
            7,
            t("Car", "Voiture"),
            "Car",
            r##"<path fill-rule="evenodd" d="M6 5h12l2.5 6H22a1 1 0 011 1v5h-2v2h-3v-2H6v2H3v-2H1v-5a1 1 0 011-1h1.5zm1.3 2L5.7 11h12.6l-1.6-4zM5.5 13a1.5 1.5 0 100 3 1.5 1.5 0 000-3zm13 0a1.5 1.5 0 100 3 1.5 1.5 0 000-3z"/>"##,
        ),
        // plane
        icon(
            7,
            t("Airplane", "Avion"),
            "Airplane",
            r##"<path d="M12 2c1 0 1.6 1 1.6 2.5V9l8.4 5v2l-8.4-2.5V18l2.4 2v1.5L12 20.5l-4 1V20l2.4-2v-4.5L2 16v-2l8.4-5V4.5C10.4 3 11 2 12 2z"/>"##,
        ),
        // check
        icon(
            8,
            t("Check mark", "Coche"),
            "Check mark",
            r##"<path d="M9 16.2L4.8 12 3 13.8l6 6 12-12L19.2 6z"/>"##,
        ),
        // cross
        icon(
            8,
            t("Cross mark", "Croix"),
            "Cross mark",
            r##"<path d="M6.4 4.6L12 10.2l5.6-5.6 1.8 1.8L13.8 12l5.6 5.6-1.8 1.8L12 13.8l-5.6 5.6-1.8-1.8L10.2 12 4.6 6.4z"/>"##,
        ),
        // warning
        icon(
            8,
            t("Warning", "Avertissement"),
            "Warning",
            r##"<path fill-rule="evenodd" d="M12 2l11 19H1zm-1 7v6h2V9zm0 7.5v2h2v-2z"/>"##,
        ),
        // info
        icon(
            8,
            t("Information", "Information"),
            "Information",
            r##"<path fill-rule="evenodd" d="M12 2a10 10 0 100 20 10 10 0 000-20zm-1 5h2v2h-2zm0 4h2v6h-2z"/>"##,
        ),
        // star
        icon(
            8,
            t("Star", "Étoile"),
            "Star",
            r##"<path d="M12 2l2.9 6.3 6.9.8-5.1 4.7 1.4 6.8L12 17.2l-6.1 3.4 1.4-6.8L2.2 9.1l6.9-.8z"/>"##,
        ),
        // flag
        icon(
            8,
            t("Flag", "Drapeau"),
            "Flag",
            r##"<path d="M5 2h1.5v1H19l-3 4.5 3 4.5H6.5v10H5z"/>"##,
        ),
        // bookmark
        icon(
            8,
            t("Bookmark", "Signet"),
            "Bookmark",
            r##"<path d="M6 2h12v20l-6-4.5L6 22z"/>"##,
        ),
        // key
        icon(
            8,
            t("Key", "Clé"),
            "Key",
            r##"<path fill-rule="evenodd" d="M8 7a5 5 0 014.9 4H22v3h-2v3h-3v-3h-4.1A5 5 0 118 7zm0 3a2 2 0 100 4 2 2 0 000-4z"/>"##,
        ),
        // lock
        icon(
            8,
            t("Lock", "Cadenas"),
            "Lock",
            r##"<path fill-rule="evenodd" d="M12 2a5 5 0 015 5v3h1a1 1 0 011 1v10a1 1 0 01-1 1H6a1 1 0 01-1-1V11a1 1 0 011-1h1V7a5 5 0 015-5zm0 2a3 3 0 00-3 3v3h6V7a3 3 0 00-3-3zm0 10a1.5 1.5 0 00-1 2.6V19h2v-2.4a1.5 1.5 0 00-1-2.6z"/>"##,
        ),
        // magnifier
        icon(
            8,
            t("Magnifying glass", "Loupe"),
            "Magnifying glass",
            r##"<path fill-rule="evenodd" d="M10 2a8 8 0 016.3 12.9l5.4 5.4-1.4 1.4-5.4-5.4A8 8 0 1110 2zm0 2.5a5.5 5.5 0 100 11 5.5 5.5 0 000-11z"/>"##,
        ),
        // medical
        icon(
            8,
            t("Medical cross", "Croix médicale"),
            "Medical cross",
            r##"<path d="M9 3h6v6h6v6h-6v6H9v-6H3V9h6z"/>"##,
        ),
        // arrow-right
        icon(
            9,
            t("Arrow right", "Flèche vers la droite"),
            "Arrow right",
            r##"<path d="M3 10.5h11V6l7 6-7 6v-4.5H3z"/>"##,
        ),
        // arrow-up
        icon(
            9,
            t("Arrow up", "Flèche vers le haut"),
            "Arrow up",
            r##"<path d="M10.5 21V10H6l6-7 6 7h-4.5v11z"/>"##,
        ),
        // arrow-both
        icon(
            9,
            t("Arrow left and right", "Flèche gauche et droite"),
            "Arrow left and right",
            r##"<path d="M2 12l5-5v3.5h10V7l5 5-5 5v-3.5H7V17z"/>"##,
        ),
        // book
        icon(
            10,
            t("Open book", "Livre ouvert"),
            "Open book",
            r##"<path d="M3 4c3-1 6-1 8.25.5V20C9 18.5 6 18.5 3 19.5zM21 4c-3-1-6-1-8.25.5V20c2.25-1.5 5.25-1.5 8.25-.5z"/>"##,
        ),
        // graduation
        icon(
            10,
            t("Graduation cap", "Toque de diplômé"),
            "Graduation cap",
            r##"<path d="M12 4l11 5-11 5L1 9z"/><path d="M5 12.5l7 3.2 7-3.2V17c-2 2-4.5 3-7 3s-5-1-7-3z"/><path d="M21 10h1.5v6H21z"/>"##,
        ),
        // pencil
        icon(
            10,
            t("Pencil", "Crayon"),
            "Pencil",
            r##"<path d="M17 2l5 5-2.5 2.5-5-5zM13 6l5 5L8 21H3v-5z"/>"##,
        ),
        // music
        icon(
            11,
            t("Music note", "Note de musique"),
            "Music note",
            r##"<path d="M9 3h11v3.5h-9V17a3.5 3.5 0 11-2-3.2z"/>"##,
        ),
        // picture
        icon(
            11,
            t("Picture", "Image"),
            "Picture",
            r##"<path fill-rule="evenodd" d="M2 4h20v16H2zm2 2v9.5l4.5-4.5 4 4 2.5-2.5 5 5V6zm12.5 1.5a1.75 1.75 0 110 3.5 1.75 1.75 0 010-3.5z"/>"##,
        ),
        // coffee
        icon(
            12,
            t("Cup of coffee", "Tasse de café"),
            "Cup of coffee",
            r##"<path fill-rule="evenodd" d="M3 8h14v2h2a3 3 0 010 6h-2.3A6 6 0 0111 20H9a6 6 0 01-6-6zm14 4v2h2a1 1 0 000-2z"/><path d="M6 2h1.5v4H6zM9.25 2h1.5v4h-1.5zM12.5 2H14v4h-1.5z"/>"##,
        ),
        // cutlery
        icon(
            12,
            t("Fork and knife", "Fourchette et couteau"),
            "Fork and knife",
            r##"<path d="M5 2h1.2v6H7V2h1.2v6H9V2h1.2v7a2.5 2.5 0 01-1.7 2.4V22h-2V11.4A2.5 2.5 0 014.8 9V2z"/><path d="M15 2c2.5 1 4 4 4 8h-2v12h-2z"/>"##,
        ),
    ]
}
