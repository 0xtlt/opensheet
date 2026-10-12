//! The `<extLst>` of a worksheet: the one place where every feature added
//! to the format after 2007 hangs its element, each in an `<ext>` of its
//! own. Features hand their `<ext>` here and the list comes out in the
//! order Excel writes it, whoever asked first.

/// The extensions of a worksheet Excel knows, in the order it writes
/// them: conditional formats, data validations, sparklines, the slicers
/// of PivotTables, the timelines, the slicers of tables.
const ORDER: [&str; 6] = [
    "{78C0D931-6437-407d-A8EE-F0AAD7539E65}",
    "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}",
    "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}",
    "{A8765BA9-456A-4dab-B4F3-ACF838C121DE}",
    "{7E03D99C-DC04-49d9-9315-930204A7B6E9}",
    "{3A4CF648-6AED-40f4-86FF-DC5316D8AED3}",
];

/// The `<ext>` elements of one worksheet, as the features give them.
#[derive(Default)]
pub(crate) struct ExtList {
    entries: Vec<(usize, String)>,
}

impl ExtList {
    /// Add what a feature wrote: nothing, or whole `<ext uri="…">`
    /// elements.
    pub fn add(&mut self, ext: String) {
        if ext.is_empty() {
            return;
        }
        let rank = ORDER
            .iter()
            .position(|uri| ext.to_lowercase().contains(&uri.to_lowercase()))
            .unwrap_or(ORDER.len());
        self.entries.push((rank, ext));
    }

    /// The `<extLst>` element; empty when no feature has an extension.
    pub fn finish(mut self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        // A stable sort: extensions unknown here keep the order they came in.
        self.entries.sort_by_key(|(rank, _)| *rank);
        let body: String = self.entries.into_iter().map(|(_, ext)| ext).collect();
        format!("<extLst>{body}</extLst>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_come_out_in_the_order_of_excel() {
        let mut list = ExtList::default();
        list.add(String::new());
        assert_eq!(ExtList::default().finish(), "");
        list.add("<ext uri=\"{A8765BA9-456A-4dab-B4F3-ACF838C121DE}\"/>".to_string());
        list.add("<ext uri=\"{unknown}\"/>".to_string());
        list.add("<ext uri=\"{78C0D931-6437-407d-A8EE-F0AAD7539E65}\"/>".to_string());
        assert_eq!(
            list.finish(),
            "<extLst><ext uri=\"{78C0D931-6437-407d-A8EE-F0AAD7539E65}\"/><ext uri=\"{A8765BA9-456A-4dab-B4F3-ACF838C121DE}\"/><ext uri=\"{unknown}\"/></extLst>"
        );
    }
}
