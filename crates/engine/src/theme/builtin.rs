//! The themes, colour sets and font pairs Excel 365 for Mac offers, read
//! from its own resources (`Office Themes/*.thmx`, `Theme Colors/*.xml`,
//! `Theme Fonts/*.xml`) and kept here as data.
//!
//! Colours are in the order of a theme part: dark 1, light 1, dark 2,
//! light 2, accents 1 to 6, hyperlink, followed hyperlink.

/// A theme of the Themes gallery.
pub struct BuiltinTheme {
    /// The name files carry (`<a:theme name>`), in English.
    pub name: &'static str,
    /// The name of its colours and of its fonts in the Colors and Fonts
    /// menus, and in the gallery.
    pub scheme: &'static str,
    pub colors: [u32; 12],
    pub fonts: BuiltinFonts,
}

/// A pair of fonts: headings, body text.
pub struct BuiltinFonts {
    pub name: &'static str,
    pub major: &'static str,
    pub minor: &'static str,
    /// The PANOSE classification of each, which theme parts state.
    pub major_panose: &'static str,
    pub minor_panose: &'static str,
}

/// A colour set of the Colors menu.
pub struct BuiltinColors {
    pub name: &'static str,
    pub colors: [u32; 12],
}

const fn fonts(
    name: &'static str,
    major: &'static str,
    major_panose: &'static str,
    minor: &'static str,
    minor_panose: &'static str,
) -> BuiltinFonts {
    BuiltinFonts {
        name,
        major,
        minor,
        major_panose,
        minor_panose,
    }
}

const fn theme(name: &'static str, scheme: &'static str, colors: [u32; 12], fonts: BuiltinFonts) -> BuiltinTheme {
    BuiltinTheme {
        name,
        scheme,
        colors,
        fonts,
    }
}

/// The Themes gallery, in its order. The first is the theme of a new
/// workbook.
pub const THEMES: &[BuiltinTheme] = &[
    theme(
        "Office Theme",
        "Office",
        [0x000000, 0xFFFFFF, 0x0E2841, 0xE8E8E8, 0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E, 0x467886, 0x96607D],
        fonts("Office", "Aptos Display", "02110004020202020204", "Aptos Narrow", "02110004020202020204"),
    ),
    theme(
        "Office 2013 - 2022 Theme",
        "Office 2013 - 2022",
        [0x000000, 0xFFFFFF, 0x44546A, 0xE7E6E6, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47, 0x0563C1, 0x954F72],
        fonts("Office 2013 - 2022", "Calibri Light", "020F0302020204030204", "Calibri", "020F0502020204030204"),
    ),
    theme(
        "Facet",
        "Facet",
        [0x000000, 0xFFFFFF, 0x2C3C43, 0xEBEBEB, 0x90C226, 0x54A021, 0xE6B91E, 0xE76618, 0xC42F1A, 0x918655, 0x99CA3C, 0xB9D181],
        fonts("Facet", "Trebuchet MS", "020B0603020202020204", "Trebuchet MS", "020B0603020202020204"),
    ),
    theme(
        "Integral",
        "Integral",
        [0x000000, 0xFFFFFF, 0x335B74, 0xDFE3E5, 0x1CADE4, 0x2683C6, 0x27CED7, 0x42BA97, 0x3E8853, 0x62A39F, 0x6B9F25, 0xB26B02],
        fonts("Integral", "Tw Cen MT Condensed", "020B0606020104020203", "Tw Cen MT", "020B0602020104020603"),
    ),
    theme(
        "Ion",
        "Ion",
        [0x000000, 0xFFFFFF, 0x1E5155, 0xEBEBEB, 0xB01513, 0xEA6312, 0xE6B729, 0x6AAC90, 0x54849A, 0x9E5E9B, 0x58C1BA, 0x9DFFCB],
        fonts("Ion", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Ion Boardroom",
        "Ion Boardroom",
        [0x000000, 0xFFFFFF, 0x3B3059, 0xEBEBEB, 0xB31166, 0xE33D6F, 0xE45F3C, 0xE9943A, 0x9B6BF2, 0xD53DD0, 0x8F8F8F, 0xA5A5A5],
        fonts("Ion Boardroom", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Organic",
        "Organic",
        [0x000000, 0xFFFFFF, 0x212121, 0xDADADA, 0x83992A, 0x3C9770, 0x44709D, 0xA23C33, 0xD97828, 0xDEB340, 0xA8BF4D, 0xB4CA80],
        fonts("Organic", "Garamond", "02020404030301010803", "Garamond", "02020404030301010803"),
    ),
    theme(
        "Retrospect",
        "Retrospect",
        [0x000000, 0xFFFFFF, 0x637052, 0xCCDDEA, 0xE48312, 0xBD582C, 0x865640, 0x9B8357, 0xC2BC80, 0x94A088, 0x2998E3, 0x8C8C8C],
        fonts("Retrospect", "Calibri Light", "020F0302020204030204", "Calibri", "020F0502020204030204"),
    ),
    theme(
        "Slice",
        "Slice",
        [0x000000, 0xFFFFFF, 0x146194, 0x76DBF4, 0x052F61, 0xA50E82, 0x14967C, 0x6A9E1F, 0xE87D37, 0xC62324, 0x0D2E46, 0x356A95],
        fonts("Slice", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Wisp",
        "Wisp",
        [0x000000, 0xFFFFFF, 0x766F54, 0xE3EACF, 0xA53010, 0xDE7E18, 0x9F8351, 0x728653, 0x92AA4C, 0x6AAC91, 0xFB4A18, 0xFB9318],
        fonts("Wisp", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Atlas",
        "Atlas",
        [0x000000, 0xFFFFFF, 0x454545, 0xE0E0E0, 0xF81B02, 0xFC7715, 0xAFBF41, 0x50C49F, 0x3B95C4, 0xB560D4, 0xFC5A1A, 0xB49E74],
        fonts("Atlas", "Calibri Light", "020F0302020204030204", "Rockwell", "02060603020205020403"),
    ),
    theme(
        "Badge",
        "Badge",
        [0x000000, 0xFFFFFF, 0x2A1A00, 0xF3F3F2, 0xF8B323, 0x656A59, 0x46B2B5, 0x8CAA7E, 0xD36F68, 0x826276, 0x46B2B5, 0xA46694],
        fonts("Badge", "Impact", "020B0806030902050204", "Gill Sans MT", "020B0502020104020203"),
    ),
    theme(
        "Berlin",
        "Berlin",
        [0x000000, 0xFFFFFF, 0x9D360E, 0xE7E6E6, 0xF09415, 0xC1B56B, 0x4BAF73, 0x5AA6C0, 0xD17DF9, 0xFA7E5C, 0xFFAE3E, 0xFCC77E],
        fonts("Berlin", "Trebuchet MS", "020B0603020202020204", "Trebuchet MS", "020B0603020202020204"),
    ),
    theme(
        "Celestial",
        "Celestial",
        [0x000000, 0xFFFFFF, 0x18276C, 0xEBEBEB, 0xAC3EC1, 0x477BD1, 0x46B298, 0x90BA4C, 0xDD9D31, 0xE25247, 0xC573D2, 0xCCAEE8],
        fonts("Celestial", "Calibri Light", "020F0302020204030204", "Calibri", "020F0502020204030204"),
    ),
    theme(
        "Crop",
        "Crop",
        [0x000000, 0xFFFFFF, 0x191B0E, 0xEFEDE3, 0x8C8D86, 0xE6C069, 0x897B61, 0x8DAB8E, 0x77A2BB, 0xE28394, 0x77A2BB, 0x957A99],
        fonts("Crop", "Franklin Gothic Book", "020B0503020102020204", "Franklin Gothic Book", "020B0503020102020204"),
    ),
    theme(
        "Depth",
        "Depth",
        [0x000000, 0xFFFFFF, 0x455F51, 0x94D7E4, 0x41AEBD, 0x97E9D5, 0xA2CF49, 0x608F3D, 0xF4DE3A, 0xFCB11C, 0xFBCA98, 0xD3B86D],
        fonts("Depth", "Corbel", "020B0503020204020204", "Corbel", "020B0503020204020204"),
    ),
    theme(
        "Droplet",
        "Droplet",
        [0x000000, 0xFFFFFF, 0x355071, 0xAABED7, 0x2FA3EE, 0x4BCAAD, 0x86C157, 0xD99C3F, 0xCE6633, 0xA35DD1, 0x56BCFE, 0x97C5E3],
        fonts("Droplet", "Tw Cen MT", "020B0602020104020603", "Tw Cen MT", "020B0602020104020603"),
    ),
    theme(
        "Feathered",
        "Feathered",
        [0x000000, 0xFFFFFF, 0x121316, 0xFEFCF7, 0x606372, 0x79A8A4, 0xB2AD8F, 0xAD8082, 0xDEC18C, 0x92A185, 0x85C4D2, 0x8E8CA7],
        fonts("Feathered", "Century Schoolbook", "02040604050505020304", "Calibri", "020F0502020204030204"),
    ),
    theme(
        "Gallery",
        "Gallery",
        [0x000000, 0xFFFFFF, 0x454545, 0xDFDBD5, 0xB71E42, 0xDE478E, 0xBC72F0, 0x795FAF, 0x586EA6, 0x6892A0, 0xFA2B5C, 0xBC658E],
        fonts("Gallery", "Gill Sans MT", "020B0502020104020203", "Gill Sans MT", "020B0502020104020203"),
    ),
    theme(
        "Headlines",
        "Headlines",
        [0x000000, 0xFFFFFF, 0x1D1A1D, 0xF5F5F5, 0x439EB7, 0xE28B55, 0xDCB64D, 0x4CA198, 0x835B82, 0x645135, 0x439EB7, 0x835B82],
        fonts("Headlines", "Century Schoolbook", "02040604050505020304", "Corbel", "020B0503020204020204"),
    ),
    theme(
        "Madison",
        "Madison",
        [0x000000, 0xFFFFFF, 0x1F2D29, 0xC5FAEB, 0xA1D68B, 0x5EC795, 0x4DADCF, 0xCDB756, 0xE29C36, 0x8EC0C1, 0x6D9D9B, 0x6D8583],
        fonts("Madison", "Arial", "020B0604020202020204", "Arial", "020B0604020202020204"),
    ),
    theme(
        "Main Event",
        "Main Event",
        [0x000000, 0xFFFFFF, 0x424242, 0xC8C8C8, 0xB80E0F, 0xA6987D, 0x7F9A71, 0x64969F, 0x9B75B2, 0x80737A, 0xF21213, 0xB6A394],
        fonts("Main Event", "Impact", "020B0806030902050204", "Impact", "020B0806030902050204"),
    ),
    theme(
        "Mesh",
        "Mesh",
        [0x000000, 0xFFFFFF, 0x363D46, 0xEBEBEB, 0x6F6F6F, 0xBFBFA5, 0xDCD084, 0xE7BF5F, 0xE9A039, 0xCF7133, 0xF28943, 0xF1B76C],
        fonts("Mesh", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Parallax",
        "Parallax",
        [0x000000, 0xFFFFFF, 0x212121, 0xCDD0D1, 0x30ACEC, 0x80C34F, 0xE29D3E, 0xD64A3B, 0xD64787, 0xA666E1, 0x3085ED, 0x82B6F4],
        fonts("Parallax", "Corbel", "020B0503020204020204", "Corbel", "020B0503020204020204"),
    ),
    theme(
        "Parcel",
        "Parcel",
        [0x000000, 0xFFFFFF, 0x4A5356, 0xE8E3CE, 0xF6A21D, 0x9BAFB5, 0xC96731, 0x9CA383, 0x87795D, 0xA0988C, 0x00B0F0, 0x738F97],
        fonts("Parcel", "Gill Sans MT", "020B0502020104020203", "Gill Sans MT", "020B0502020104020203"),
    ),
    theme(
        "Savon",
        "Savon",
        [0x000000, 0xFFFFFF, 0x1485A4, 0xE3DED1, 0x1CADE4, 0x2683C6, 0x27CED7, 0x42BA97, 0x3E8853, 0x62A39F, 0xF49100, 0x739D9B],
        fonts("Savon", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "Vapor Trail",
        "Vapor Trail",
        [0x000000, 0xFFFFFF, 0x454545, 0xDADADA, 0xDF2E28, 0xFE801A, 0xE9BF35, 0x81BB42, 0x32C7A9, 0x4A9BDC, 0xF0532B, 0xF38B53],
        fonts("Vapor Trail", "Century Gothic", "020B0502020202020204", "Century Gothic", "020B0502020202020204"),
    ),
    theme(
        "View",
        "View",
        [0x000000, 0xFFFFFF, 0x46464A, 0xD6D3CC, 0x6F6F74, 0x92A9B9, 0xA7B789, 0xB9A489, 0x8D6374, 0x9B7362, 0x67AABF, 0xABAFA5],
        fonts("View", "Century Schoolbook", "02040604050505020304", "Century Schoolbook", "02040604050505020304"),
    ),
    theme(
        "Wood Type",
        "Wood Type",
        [0x000000, 0xFFFFFF, 0x696464, 0xE9E5DC, 0xD34817, 0x9B2D1F, 0xA28E6A, 0x956251, 0x918485, 0x855D5D, 0xCC9900, 0x96A9A9],
        fonts("Wood Type", "Rockwell Condensed", "02060603050405020104", "Rockwell", "02060603020205020403"),
    ),
];

/// The colour sets of the Colors menu after the first, "Office", which
/// is the colours of the first theme.
pub const COLOR_SETS: &[BuiltinColors] = &[
    BuiltinColors {
        name: "Office 2013 - 2022",
        colors: [0x000000, 0xFFFFFF, 0x44546A, 0xE7E6E6, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47, 0x0563C1, 0x954F72],
    },
    BuiltinColors {
        name: "Office 2007 - 2010",
        colors: [0x000000, 0xFFFFFF, 0x1F497D, 0xEEECE1, 0x4F81BD, 0xC0504D, 0x9BBB59, 0x8064A2, 0x4BACC6, 0xF79646, 0x0000FF, 0x800080],
    },
    BuiltinColors {
        name: "Grayscale",
        colors: [0x000000, 0xFFFFFF, 0x000000, 0xF8F8F8, 0xDDDDDD, 0xB2B2B2, 0x969696, 0x808080, 0x5F5F5F, 0x4D4D4D, 0x5F5F5F, 0x919191],
    },
    BuiltinColors {
        name: "Blue Warm",
        colors: [0x000000, 0xFFFFFF, 0x242852, 0xACCBF9, 0x4A66AC, 0x629DD1, 0x297FD5, 0x7F8FA9, 0x5AA2AE, 0x9D90A0, 0x9454C3, 0x3EBBF0],
    },
    BuiltinColors {
        name: "Blue",
        colors: [0x000000, 0xFFFFFF, 0x17406D, 0xDBEFF9, 0x0F6FC6, 0x009DD9, 0x0BD0D9, 0x10CF9B, 0x7CCA62, 0xA5C249, 0xF49100, 0x85DFD0],
    },
    BuiltinColors {
        name: "Blue II",
        colors: [0x000000, 0xFFFFFF, 0x335B74, 0xDFE3E5, 0x1CADE4, 0x2683C6, 0x27CED7, 0x42BA97, 0x3E8853, 0x62A39F, 0x6EAC1C, 0xB26B02],
    },
    BuiltinColors {
        name: "Blue Green",
        colors: [0x000000, 0xFFFFFF, 0x373545, 0xCEDBE6, 0x3494BA, 0x58B6C0, 0x75BDA7, 0x7A8C8E, 0x84ACB6, 0x2683C6, 0x6B9F25, 0x9F6715],
    },
    BuiltinColors {
        name: "Green",
        colors: [0x000000, 0xFFFFFF, 0x455F51, 0xE3DED1, 0x549E39, 0x8AB833, 0xC0CF3A, 0x029676, 0x4AB5C4, 0x0989B1, 0x6B9F25, 0xBA6906],
    },
    BuiltinColors {
        name: "Green Yellow",
        colors: [0x000000, 0xFFFFFF, 0x455F51, 0xE2DFCC, 0x99CB38, 0x63A537, 0x37A76F, 0x44C1A3, 0x4EB3CF, 0x51C3F9, 0xEE7B08, 0x977B2D],
    },
    BuiltinColors {
        name: "Yellow",
        colors: [0x000000, 0xFFFFFF, 0x39302A, 0xE5DEDB, 0xFFCA08, 0xF8931D, 0xCE8D3E, 0xEC7016, 0xE64823, 0x9C6A6A, 0x2998E3, 0x7F723D],
    },
    BuiltinColors {
        name: "Yellow Orange",
        colors: [0x000000, 0xFFFFFF, 0x4E3B30, 0xFBEEC9, 0xF0A22E, 0xA5644E, 0xB58B80, 0xC3986D, 0xA19574, 0xC17529, 0xAD1F1F, 0xFFC42F],
    },
    BuiltinColors {
        name: "Orange",
        colors: [0x000000, 0xFFFFFF, 0x637052, 0xCCDDEA, 0xE48312, 0xBD582C, 0x865640, 0x9B8357, 0xC2BC80, 0x94A088, 0x2998E3, 0x8C8C8C],
    },
    BuiltinColors {
        name: "Orange Red",
        colors: [0x000000, 0xFFFFFF, 0x696464, 0xE9E5DC, 0xD34817, 0x9B2D1F, 0xA28E6A, 0x956251, 0x918485, 0x855D5D, 0xCC9900, 0x96A9A9],
    },
    BuiltinColors {
        name: "Red Orange",
        colors: [0x000000, 0xFFFFFF, 0x505046, 0xEEECE1, 0xE84C22, 0xFFBD47, 0xB64926, 0xFF8427, 0xCC9900, 0xB22600, 0xCC9900, 0x666699],
    },
    BuiltinColors {
        name: "Red",
        colors: [0x000000, 0xFFFFFF, 0x323232, 0xE5C243, 0xA5300F, 0xD55816, 0xE19825, 0xB19C7D, 0x7F5F52, 0xB27D49, 0x6B9F25, 0xB26B02],
    },
    BuiltinColors {
        name: "Red Violet",
        colors: [0x000000, 0xFFFFFF, 0x454551, 0xD8D9DC, 0xE32D91, 0xC830CC, 0x4EA6DC, 0x4775E7, 0x8971E1, 0xD54773, 0x6B9F25, 0x8C8C8C],
    },
    BuiltinColors {
        name: "Violet",
        colors: [0x000000, 0xFFFFFF, 0x373545, 0xDCD8DC, 0xAD84C6, 0x8784C7, 0x5D739A, 0x6997AF, 0x84ACB6, 0x6F8183, 0x69A020, 0x8C8C8C],
    },
    BuiltinColors {
        name: "Violet II",
        colors: [0x000000, 0xFFFFFF, 0x632E62, 0xEAE5EB, 0x92278F, 0x9B57D3, 0x755DD9, 0x665EB8, 0x45A5ED, 0x5982DB, 0x0066FF, 0x666699],
    },
    BuiltinColors {
        name: "Median",
        colors: [0x000000, 0xFFFFFF, 0x775F55, 0xEBDDC3, 0x94B6D2, 0xDD8047, 0xA5AB81, 0xD8B25C, 0x7BA79D, 0x968C8C, 0xF7B615, 0x704404],
    },
    BuiltinColors {
        name: "Paper",
        colors: [0x000000, 0xFFFFFF, 0x444D26, 0xFEFAC9, 0xA5B592, 0xF3A447, 0xE7BC29, 0xD092A7, 0x9C85C0, 0x809EC2, 0x8E58B6, 0x7F6F6F],
    },
    BuiltinColors {
        name: "Marquee",
        colors: [0x000000, 0xFFFFFF, 0x5E5E5E, 0xDDDDDD, 0x418AB3, 0xA6B727, 0xF69200, 0x838383, 0xFEC306, 0xDF5327, 0xF59E00, 0xB2B2B2],
    },
    BuiltinColors {
        name: "Slipstream",
        colors: [0x000000, 0xFFFFFF, 0x212745, 0xB4DCFA, 0x4E67C8, 0x5ECCF3, 0xA7EA52, 0x5DCEAF, 0xFF8021, 0xF14124, 0x56C7AA, 0x59A8D1],
    },
    BuiltinColors {
        name: "Aspect",
        colors: [0x000000, 0xFFFFFF, 0x323232, 0xE3DED1, 0xF07F09, 0x9F2936, 0x1B587C, 0x4E8542, 0x604878, 0xC19859, 0x6B9F25, 0xB26B02],
    },
];

/// The font pairs of the Fonts menu after the first, "Office", which is
/// the fonts of the first theme.
pub const FONT_PAIRS: &[BuiltinFonts] = &[
    fonts("Office 2013 - 2022", "Calibri Light", "020F0302020204030204", "Calibri", "020F0502020204030204"),
    fonts("Office 2007 - 2010", "Cambria", "02040503050406030204", "Calibri", "020F0502020204030204"),
    fonts("Calibri", "Calibri", "020F0502020204030204", "Calibri", "020F0502020204030204"),
    fonts("Arial", "Arial", "020B0604020202020204", "Arial", "020B0604020202020204"),
    fonts("Corbel", "Corbel", "020B0503020204020204", "Corbel", "020B0503020204020204"),
    fonts("Candara", "Candara", "020E0502030303020204", "Candara", "020E0502030303020204"),
    fonts("Franklin Gothic", "Franklin Gothic Medium", "020B0603020102020204", "Franklin Gothic Book", "020B0503020102020204"),
    fonts("Century Gothic", "Century Gothic", "020F0302020204030204", "Century Gothic", "020F0302020204030204"),
    fonts("Tw Cen MT", "Tw Cen MT", "020B0602020104020603", "Tw Cen MT", "020B0602020104020603"),
    fonts("Cambria", "Cambria", "02040503050406030204", "Cambria", "02040503050406030204"),
    fonts("Garamond", "Garamond", "02020404030301010803", "Garamond", "02020404030301010803"),
    fonts("Georgia", "Georgia", "02040502050405020303", "Georgia", "02040502050405020303"),
    fonts("Century Schoolbook", "Century Schoolbook", "02040604050505020304", "Century Schoolbook", "02040604050505020304"),
    fonts("Calibri-Cambria", "Calibri", "020F0502020204030204", "Cambria", "02040503050406030204"),
    fonts("Calibri Light-Constantia", "Calibri Light", "020F0302020204030204", "Constantia", "02030602050306030303"),
    fonts("Constantia-Franklin Gothic Book", "Constantia", "02030602050306030303", "Franklin Gothic Book", "020B0503020102020204"),
    fonts("Tw Cen MT-Rockwell", "Tw Cen MT", "020B0602020104020603", "Rockwell", "02060603020205020403"),
    fonts("Times New Roman-Arial", "Times New Roman", "02020603050405020304", "Arial", "020B0604020202020204"),
    fonts("Arial-Times New Roman", "Arial", "020B0604020202020204", "Times New Roman", "02020603050405020304"),
    fonts("Arial Black-Arial", "Arial Black", "020B0A04020102020204", "Arial", "020B0604020202020204"),
    fonts("Century Gothic-Palatino Linotype", "Century Gothic", "020B0502020202020204", "Palatino Linotype", "02040502050505030304"),
    fonts("Garamond-Trebuchet MS", "Garamond", "02020404030301010803", "Trebuchet MS", "020B0603020202020204"),
    fonts("Consolas-Verdana", "Consolas", "020B0609020204030204", "Verdana", "020B0604030504040204"),
    fonts("Gill Sans MT", "Gill Sans MT", "020B0502020104020203", "Gill Sans MT", "020B0502020104020203"),
    fonts("Trebuchet MS", "Trebuchet MS", "020B0603020202020204", "Trebuchet MS", "020B0603020202020204"),
];

/// The effects of the Effects menu, in its order. They are names only:
/// nothing in this application draws with the fills, lines and shadows an
/// effect scheme defines.
pub const EFFECTS: &[&str] = &[
    "Office",
    "Office 2007 - 2010",
    "Subtle Solids",
    "Banded Edge",
    "Smokey Glass",
    "Glow Edge",
    "Grunge Texture",
    "Frosted Glass",
    "Top Shadow",
    "Inset",
    "Milk Glass",
    "Riblet",
    "Reflection",
    "Extreme Shadow",
    "Glossy",
];

/// The PANOSE classification of a font of the lists above, by its name.
pub fn panose(typeface: &str) -> Option<&'static str> {
    THEMES
        .iter()
        .map(|theme| &theme.fonts)
        .chain(FONT_PAIRS)
        .flat_map(|pair| [(pair.major, pair.major_panose), (pair.minor, pair.minor_panose)])
        .find(|(name, panose)| name.eq_ignore_ascii_case(typeface) && !panose.is_empty())
        .map(|(_, panose)| panose)
}
