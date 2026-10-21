//! The extension list of a worksheet (`<extLst>`, its last child): what
//! later versions of the format added, one `<ext>` per feature. Every
//! feature that has one hands it to `SheetExtensions`, which writes them
//! all in one list, in the order Excel does.

/// The `uri` of each `<ext>` of a worksheet, in the order Excel writes
/// them. A feature that adds an extension adds its uri here.
const ORDER: [&str; 6] = [
    // x14:conditionalFormattings
    "{78C0D931-6437-407d-A8EE-F0AAD7539E65}",
    // x14:dataValidations
    "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}",
    // x14:sparklineGroups
    "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}",
    // x14:slicerList
    "{A8765BA9-456A-4dab-B4F3-ACF838C121DE}",
    // x15:timelineRefs
    "{7E03D99C-DC04-49d9-9315-930204A7B6E9}",
    // x14:slicerList of the slicers of tables
    "{3A4CF648-6AED-40f4-86FF-DC5316D8AED3}",
];

/// The `<ext>` elements of a worksheet being written.
#[derive(Default)]
pub(crate) struct SheetExtensions {
    /// (place in `ORDER`, the element)
    entries: Vec<(usize, String)>,
}

impl SheetExtensions {
    /// Add a complete `<ext … uri="{…}">…</ext>` element; an empty
    /// string (a feature with nothing to say) is left out. Extensions
    /// whose uri is not in `ORDER` go last, in the order they came.
    pub(crate) fn push(&mut self, ext: String) {
        if ext.is_empty() {
            return;
        }
        let head = ext.split('>').next().unwrap_or("");
        let rank = ORDER
            .iter()
            .position(|uri| head.contains(uri))
            .unwrap_or(ORDER.len());
        self.entries.push((rank, ext));
    }

    /// Append the `<extLst>` (nothing when there is no extension).
    pub(crate) fn write(mut self, x: &mut String) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.sort_by_key(|(rank, _)| *rank);
        x.push_str("<extLst>");
        for (_, ext) in &self.entries {
            x.push_str(ext);
        }
        x.push_str("</extLst>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_come_out_in_excels_order() {
        let mut list = SheetExtensions::default();
        list.push("<ext uri=\"{unknown}\"/>".into());
        list.push(format!(
            "<ext xmlns:x14=\"ns\" uri=\"{}\"><x14:sparklineGroups/></ext>",
            ORDER[2]
        ));
        list.push(String::new());
        list.push(format!("<ext uri=\"{}\"/>", ORDER[0]));
        let mut x = String::new();
        list.write(&mut x);
        let places: Vec<usize> = [ORDER[0], ORDER[2], "{unknown}"]
            .iter()
            .map(|uri| x.find(uri).unwrap())
            .collect();
        assert!(places[0] < places[1] && places[1] < places[2], "{x}");
        assert!(x.starts_with("<extLst><ext") && x.ends_with("</extLst>"));
        let mut none = String::new();
        SheetExtensions::default().write(&mut none);
        assert_eq!(none, "");
    }
}
