//! Vector pictures: Excel keeps the SVG document of an icon or of an
//! inserted SVG file beside a PNG image of it. The picture names the PNG
//! as any picture does, and the document in an extension of its blip
//! (`asvg:svgBlip`), which older applications pass over.

use super::*;

const SVG_EXTENSION: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";
const NS_SVG: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";
const LOCAL_DPI_EXTENSION: &str = "{28A0092B-C50C-407E-A947-70E740481C1C}";
const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";

/// The extension list of a blip that has an SVG document, as Excel
/// writes it. `vector` is the relationship to the document.
pub(super) fn push_extension(x: &mut String, vector: usize) {
    w!(
        x,
        "<a:extLst><a:ext uri=\"{LOCAL_DPI_EXTENSION}\"><a14:useLocalDpi xmlns:a14=\"{NS_A14}\" val=\"0\"/></a:ext><a:ext uri=\"{SVG_EXTENSION}\"><asvg:svgBlip xmlns:asvg=\"{NS_SVG}\" r:embed=\"rId{vector}\"/></a:ext></a:extLst>"
    );
}

/// The relationship to the SVG document a blip names, if it has one.
/// Consumes the blip.
pub(super) fn parse_extension<'a>(r: &mut Reader<'a>, blip: &Tag<'a>) -> Option<String> {
    let mut vector = None;
    each_child(r, blip, |r, list| {
        if list.name != "extLst" {
            r.skip(list);
            return;
        }
        each_child(r, list, |r, extension| {
            each_child(r, extension, |r, t| {
                if t.name == "svgBlip" {
                    vector = t.attr_local("embed").map(|id| id.into_owned()).filter(|id| !id.is_empty());
                }
                r.skip(t);
            });
        });
    });
    vector
}
