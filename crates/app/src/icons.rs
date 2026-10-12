//! Embedded monochrome icons, served to GPUI's `svg()` element as assets
//! under `icons/<name>.svg`.

use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

/// (name, SVG body on a 16×16 canvas, stroked).
const ICONS: &[(&str, &str)] = &[
    ("paste", r#"<rect x="3.5" y="2.5" width="9" height="11.5" rx="1"/><rect x="5.5" y="1.2" width="5" height="2.6" rx="0.8"/><path d="M6 7.5H10M6 10H10"/>"#),
    ("cut", r#"<circle cx="4.5" cy="11.8" r="1.9"/><circle cx="11.5" cy="11.8" r="1.9"/><path d="M5.6 10.2 11 2M10.4 10.2 5 2"/>"#),
    ("copy", r#"<rect x="5.5" y="4.5" width="8" height="9.5" rx="1"/><path d="M3 11.5V3.5a1 1 0 0 1 1-1h6"/>"#),
    ("brush", r#"<path d="M3 2.5h8v3.5H3zM11 4h2.2v4.2H8v2"/><rect x="7" y="10.2" width="2" height="4" rx="0.5"/>"#),
    ("undo", r#"<path d="M3.5 6.5h6a3.5 3.5 0 0 1 0 7H6.5"/><path d="M6 3.8 3.3 6.5 6 9.2"/>"#),
    ("redo", r#"<path d="M12.5 6.5h-6a3.5 3.5 0 0 0 0 7H9.5"/><path d="M10 3.8l2.7 2.7L10 9.2"/>"#),
    ("save", r#"<path d="M2.5 2.5h9l2 2v9h-11z"/><path d="M5 2.5v3.5h5V2.5M4.5 13.5V9h7v4.5"/>"#),
    ("chevron-down", r#"<path d="M4 6l4 4 4-4"/>"#),
    ("chevron-up", r#"<path d="M4 10l4-4 4 4"/>"#),
    ("chevron-left", r#"<path d="M10 4 6 8l4 4"/>"#),
    ("chevron-right", r#"<path d="M6 4l4 4-4 4"/>"#),
    ("caret-down", r#"<path d="M5 6.5 8 9.5l3-3z" fill="black"/>"#),
    ("plus", r#"<path d="M8 3v10M3 8h10"/>"#),
    ("minus", r#"<path d="M3 8h10"/>"#),
    ("close", r#"<path d="M4 4l8 8M12 4l-8 8"/>"#),
    ("check", r#"<path d="M3.5 8.5l3 3 6-7"/>"#),
    ("search", r#"<circle cx="7" cy="7" r="4"/><path d="M10 10l3.5 3.5"/>"#),
    ("more", r#"<circle cx="3.5" cy="8" r="0.9" fill="black"/><circle cx="8" cy="8" r="0.9" fill="black"/><circle cx="12.5" cy="8" r="0.9" fill="black"/>"#),
    ("borders", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M8 2.5v11M2.5 8h11"/>"#),
    ("border-bottom", r#"<path d="M2.5 13.5h11" stroke-width="2"/><path d="M2.5 2.5h11v8M2.5 2.5v8M8 2.5v8M2.5 7h11" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-top", r#"<path d="M2.5 2.5h11" stroke-width="2"/><path d="M2.5 5.5v8h11v-8M8 5.5v8M2.5 9h11" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-left", r#"<path d="M2.5 2.5v11" stroke-width="2"/><path d="M5.5 2.5h8v11h-8M5.5 8h8M9.5 2.5v11" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-right", r#"<path d="M13.5 2.5v11" stroke-width="2"/><path d="M10.5 2.5h-8v11h8M2.5 8h8M6.5 2.5v11" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-none", r#"<path d="M2.5 2.5h11v11h-11zM8 2.5v11M2.5 8h11" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-outside", r#"<rect x="2.5" y="2.5" width="11" height="11" stroke-width="1.6"/><path d="M8 4v8M4 8h8" stroke-dasharray="1 1.6" stroke-width="0.8"/>"#),
    ("border-thick", r#"<rect x="2.5" y="2.5" width="11" height="11" stroke-width="2.4"/>"#),
    ("fill", r#"<path d="M3 8.6 8 3.6l4.4 4.4-5 5z"/><path d="M5.6 6 8 3.6V1.8"/><path d="M13.2 10.4q1.3 2 0 2.8-1.3-.8 0-2.8z" fill="black"/>"#),
    ("align-left", r#"<path d="M2.5 4h11M2.5 7h7M2.5 10h11M2.5 13h7"/>"#),
    ("align-center", r#"<path d="M2.5 4h11M4.5 7h7M2.5 10h11M4.5 13h7"/>"#),
    ("align-right", r#"<path d="M2.5 4h11M6.5 7h7M2.5 10h11M6.5 13h7"/>"#),
    ("align-top", r#"<path d="M2.5 2.5h11"/><path d="M4.5 5.5h7M4.5 8.5h7" stroke-width="1.6"/>"#),
    ("align-middle", r#"<path d="M2.5 8h11" stroke-width="0.8"/><path d="M4.5 5.5h7M4.5 10.5h7" stroke-width="1.6"/>"#),
    ("align-bottom", r#"<path d="M2.5 13.5h11"/><path d="M4.5 10.5h7M4.5 7.5h7" stroke-width="1.6"/>"#),
    ("wrap", r#"<path d="M2.5 4h11M2.5 8h8.7a2 2 0 0 1 0 4H8.5M2.5 12H6"/><path d="M10 10.5 8.5 12l1.5 1.5"/>"#),
    ("merge", r#"<rect x="2.5" y="4.5" width="11" height="7"/><path d="M5 8h6M6.4 6.6 5 8l1.4 1.4M9.6 6.6 11 8 9.6 9.4"/>"#),
    ("indent-dec", r#"<path d="M7 4h6.5M7 8h6.5M7 12h6.5M5 6 2.8 8 5 10"/>"#),
    ("indent-inc", r#"<path d="M7 4h6.5M7 8h6.5M7 12h6.5M2.8 6 5 8l-2.2 2"/>"#),
    ("orientation", r#"<path d="M3 13 12.5 3.5"/><path d="M8.5 3.2h4.3v4.3"/><path d="M2.5 6.5 5 4M4.5 9 7 6.5" stroke-width="0.8"/>"#),
    ("currency", r#"<rect x="1.5" y="4.5" width="13" height="7.5" rx="1"/><circle cx="8" cy="8.2" r="1.9"/><path d="M3.6 6.5v3.4M12.4 6.5v3.4" stroke-width="0.8"/>"#),
    ("inc-decimal", r#"<circle cx="2.6" cy="12.2" r="0.7" fill="black"/><ellipse cx="6.2" cy="10" rx="1.5" ry="2.5"/><ellipse cx="11" cy="10" rx="1.5" ry="2.5"/><path d="M12.5 3.5h-8M6.4 1.8 4.5 3.5l1.9 1.7"/>"#),
    ("dec-decimal", r#"<circle cx="5" cy="12.2" r="0.7" fill="black"/><ellipse cx="9" cy="10" rx="1.5" ry="2.5"/><path d="M4.5 3.5h8M10.6 1.8l1.9 1.7-1.9 1.7"/>"#),
    ("cond-format", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.5 6.2h11M2.5 9.8h11"/><path d="M4 8h6" stroke-width="2"/><path d="M4 4.4h3" stroke-width="1.4"/><path d="M4 11.6h8" stroke-width="1.4"/>"#),
    ("table", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.5 6h11M2.5 9.7h11M6.2 6v7.5M9.9 6v7.5"/><path d="M3 4.2h10" stroke-width="2.4"/>"#),
    ("cell-styles", r#"<rect x="2.5" y="3.5" width="11" height="9"/><path d="M2.5 8h11M8 3.5v9"/><path d="M9.5 10.2h2.5" stroke-width="2.2"/>"#),
    ("insert-cells", r#"<rect x="2.5" y="7" width="11" height="6.5"/><path d="M8 1.2v4.2M5.9 3.3h4.2"/>"#),
    ("delete-cells", r#"<rect x="2.5" y="7" width="11" height="6.5"/><path d="M6.3 1.6l3.4 3.4M9.7 1.6 6.3 5"/>"#),
    ("format-cells", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M5 11V9.6l4.8-4.8 1.4 1.4L6.4 11z"/>"#),
    ("fill-down", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M8 5v6M5.6 8.6 8 11l2.4-2.4"/>"#),
    ("clear", r#"<path d="M6.6 13 2.6 9l6.4-6.4 4.4 4.4-6 6z"/><path d="M5.6 6 10 10.4M6.6 13h7"/>"#),
    ("sort-filter", r#"<path d="M2.2 3h7L6.8 6.2v4.2l-2.2 1.4V6.2z"/><path d="M11.8 3v10M9.8 11l2 2 2-2"/>"#),
    ("sort-asc", r#"<path d="M11.5 3v10M9.5 11l2 2 2-2"/><path d="M2.5 7l2-4.5L6.5 7M3.2 5.6h2.6M2.7 9.5h3.6l-3.6 4h3.6"/>"#),
    ("sort-desc", r#"<path d="M11.5 3v10M9.5 11l2 2 2-2"/><path d="M2.5 13.5l2-4.5 2 4.5M3.2 12.1h2.6M2.7 2.5h3.6l-3.6 4h3.6"/>"#),
    ("filter", r#"<path d="M2.5 3h11L9.5 8v4.5l-3 1.5V8z"/>"#),
    ("freeze", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.5 6h11M6 2.5v11" stroke-width="1.8"/>"#),
    ("grid", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.5 6.2h11M2.5 9.8h11M6.2 2.5v11M9.8 2.5v11"/>"#),
    ("chart-bar", r#"<path d="M2.5 13.5h11"/><path d="M4.5 13V8M8 13V4M11.5 13v-3" stroke-width="2"/>"#),
    ("chart-bar-h", r#"<path d="M2.500 2.500v11"/><path d="M3 4.500h5M3 8h9M3 11.500h3.500" stroke-width="2"/>"#),
    ("chart-area", r#"<path d="M2.500 2.500v11h11"/><path d="M2.500 13.500V9.500l3-3 3 2.500 5-5v9.500z" fill="black"/>"#),
    ("chart-scatter", r#"<path d="M2.500 2.500v11h11"/><circle cx="5.500" cy="10" r="1" fill="black"/><circle cx="8" cy="7" r="1" fill="black"/><circle cx="10.500" cy="8.500" r="1" fill="black"/><circle cx="12" cy="4.500" r="1" fill="black"/>"#),
    ("chart-line", r#"<path d="M2.5 2.5v11h11"/><path d="M4.5 10.5 7 7l2.5 2 3.5-5"/>"#),
    ("chart-pie", r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 2.5V8l4.2 3.6"/>"#),
    ("picture", r#"<rect x="2.5" y="3.5" width="11" height="9"/><path d="M2.5 11 6 7.5l3 3 2-2 2.5 2.5"/><circle cx="10.6" cy="6" r="1"/>"#),
    ("shapes", r#"<rect x="2.5" y="6.5" width="7" height="7"/><circle cx="10" cy="6" r="3.5"/>"#),
    ("link", r#"<path d="M7 9a2.5 2.5 0 0 0 3.5 0l2-2A2.5 2.5 0 0 0 9 3.500L8.200 4.300"/><path d="M9 7a2.5 2.5 0 0 0-3.500 0l-2 2A2.500 2.500 0 0 0 7 12.500l.8-.8"/>"#),
    ("comment", r#"<path d="M2.5 3.500h11v7.500H8l-3 2.500V11H2.500z"/>"#),
    ("textbox", r#"<rect x="2.5" y="3.5" width="11" height="9"/><path d="M5.500 6h5M8 6v4.500"/>"#),
    ("pivot", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.500 6h11M6 2.500v11"/><path d="M8.500 11.500h3v-3M11.500 11.500 8.500 8.500"/>"#),
    ("function", r#"<path d="M9.500 2.500q-2.200-.4-2.600 2L5.600 12q-.4 2-2.300 1.600M4.500 6.500h4"/><path d="M9.500 8.500l3.500 4.500M13 8.500 9.500 13"/>"#),
    ("book", r#"<path d="M3 2.500h8.500a1.500 1.500 0 0 1 1.500 1.500v9.500H4.500A1.500 1.500 0 0 1 3 12z"/><path d="M3 12a1.500 1.500 0 0 1 1.500-1.500H13"/>"#),
    ("clock", r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 4.800V8l2.200 1.500"/>"#),
    ("calculator", r#"<rect x="3.500" y="2" width="9" height="12" rx="1"/><path d="M5.500 4.500h5v2h-5z"/><path d="M5.800 9.200h.1M8 9.200h.1M10.200 9.200h.1M5.800 11.500h.1M8 11.500h.1M10.200 11.500h.1" stroke-width="1.5"/>"#),
    ("tag", r#"<path d="M2.500 2.500h5l6 6-5 5-6-6z"/><circle cx="5.300" cy="5.300" r="0.9"/>"#),
    ("zoom", r#"<circle cx="7" cy="7" r="4"/><path d="M10 10l3.500 3.500M5.200 7h3.600M7 5.200v3.600"/>"#),
    ("view-normal", r#"<rect x="2.500" y="3" width="11" height="10"/><path d="M2.500 6.300h11M2.500 9.700h11M6.200 3v10M9.900 3v10"/>"#),
    ("view-layout", r#"<rect x="2.500" y="3" width="11" height="10"/><rect x="5" y="5" width="6" height="8"/>"#),
    ("view-break", r#"<rect x="2.500" y="3" width="11" height="10"/><path d="M2.500 8h11M8 3v10" stroke-dasharray="1.500 1.500"/>"#),
    ("share", r#"<path d="M5.500 6.500h-2v7h9v-7h-2M8 9.500v-8M5.600 3.800 8 1.500l2.400 2.300"/>"#),
    ("home", r#"<path d="M2.500 7.500 8 2.500l5.500 5v6h-3.700v-4H6.200v4H2.500z"/>"#),
    ("folder", r#"<path d="M2 4h4l1.500 1.500H14v7.500H2z"/>"#),
    ("new", r#"<path d="M4 2h5.500L12.500 5v9H4z"/><path d="M9.500 2v3h3"/>"#),
    ("print", r#"<path d="M4.500 6V2.500h7V6M4.500 11h-2V6h11v5h-2"/><rect x="4.500" y="9" width="7" height="4.500"/>"#),
    ("font-grow", r#"<path d="M2 13 6 3l4 10M3.400 9.500h5.200"/><path d="M11 5.500l1.800-2 1.800 2"/>"#),
    ("font-shrink", r#"<path d="M2.500 13 5.800 5 9 13M3.700 10.300H7.900"/><path d="M11 3.800l1.800 2 1.800-2"/>"#),
    ("triangle", r#"<path d="M13.500 6.500v7h-7z" fill="black" stroke="none"/>"#),
    ("eye", r#"<path d="M1.500 8q3-4.500 6.500-4.500T14.500 8q-3 4.500-6.500 4.500T1.500 8z"/><circle cx="8" cy="8" r="2"/>"#),
    ("spell", r#"<path d="M2.500 9 5 3l2.500 6M3.400 7h3.200M9 3h2.200a1.400 1.400 0 0 1 0 2.800H9zM9 5.800h2.600a1.500 1.500 0 0 1 0 3.200H9z"/><path d="M6 12.500l2 2 4.500-4.500"/>"#),
    ("lock", r#"<rect x="3.500" y="7" width="9" height="6.500" rx="1"/><path d="M5.500 7V5a2.500 2.500 0 0 1 5 0v2"/>"#),
    ("page", r#"<path d="M4 2h8v12H4z"/><path d="M6 5h4M6 7.500h4M6 10h4"/>"#),
    ("data", r#"<ellipse cx="8" cy="4" rx="5" ry="2"/><path d="M3 4v8c0 1.100 2.200 2 5 2s5-.9 5-2V4M3 8c0 1.100 2.200 2 5 2s5-.9 5-2"/>"#),
    ("refresh", r#"<path d="M13 8a5 5 0 1 1-1.500-3.500"/><path d="M13.200 2.500v3h-3"/>"#),
    ("columns", r#"<rect x="2.500" y="2.500" width="11" height="11"/><path d="M6.200 2.500v11M9.900 2.500v11"/>"#),
    ("validation", r#"<rect x="2.500" y="3.500" width="11" height="9"/><path d="M5 8l2 2 4-4.500"/>"#),
    ("trash", r#"<path d="M3 4.500h10M6 4.500v-2h4v2M4.500 4.500l.6 9h5.800l.6-9"/>"#),
    ("launcher", r#"<path d="M4 11V4h7"/><path d="M7.500 7.500 12 12M12 8.500V12H8.500"/>"#),
    ("slicer", r#"<rect x="2.500" y="2.500" width="11" height="11" rx="1"/><path d="M2.500 5.500h11M5 8h6M5 10.500h6"/>"#),
    ("note", r#"<path d="M3 2.500h10v8l-3 3H3z"/><path d="M10 13.500v-3h3"/>"#),
    ("margins", r#"<rect x="3" y="2" width="10" height="12"/><path d="M5.500 2v12M10.500 2v12M3 4.500h10M3 11.500h10" stroke-dasharray="1.200 1.200" stroke-width="0.8"/>"#),
    ("breaks", r#"<path d="M4 2h8v4H4zM4 10h8v4H4z"/><path d="M2 8h12" stroke-dasharray="1.500 1.500"/>"#),
    ("header-footer", r#"<rect x="3.500" y="2" width="9" height="12"/><path d="M5.500 4.500h5M5.500 11.500h5" stroke-width="1.600"/>"#),
    ("wordart", r#"<path d="M2.500 13 6 3l3.500 10M3.700 9.500h4.600"/><path d="M11.500 5q3 4 0 8"/>"#),
    ("smartart", r#"<rect x="2" y="6" width="4" height="4"/><rect x="10" y="2" width="4" height="4"/><rect x="10" y="10" width="4" height="4"/><path d="M6 8h2M8 4v8M8 4h2M8 12h2"/>"#),
    ("bring-forward", r#"<rect x="6" y="6" width="7.500" height="7.500"/><rect x="2.500" y="2.500" width="7.500" height="7.500" fill="black"/>"#),
    ("send-backward", r#"<rect x="2.500" y="2.500" width="7.500" height="7.500" fill="black"/><rect x="6" y="6" width="7.500" height="7.500" fill="white"/>"#),
    ("align-objects", r#"<path d="M2.500 2v12"/><rect x="4.500" y="3.500" width="8" height="3"/><rect x="4.500" y="9.500" width="5" height="3"/>"#),
    ("group-objects", r#"<rect x="3.500" y="3.500" width="5" height="5"/><rect x="7.500" y="7.500" width="5" height="5"/><path d="M1.500 1.500h13v13h-13z" stroke-dasharray="1.500 1.500" stroke-width="0.8"/>"#),
    ("crop", r#"<path d="M4.500 1.500v10h10M1.500 4.500h10v10"/>"#),
    ("border-inside", r#"<path d="M2.500 2.500h11v11h-11z" stroke-dasharray="1 1.600" stroke-width="0.8"/><path d="M8 2.500v11M2.500 8h11" stroke-width="1.600"/>"#),
    ("border-inner-h", r#"<path d="M2.500 2.500h11v11h-11zM8 2.500v11" stroke-dasharray="1 1.600" stroke-width="0.8"/><path d="M2.500 8h11" stroke-width="1.800"/>"#),
    ("border-inner-v", r#"<path d="M2.500 2.500h11v11h-11zM2.500 8h11" stroke-dasharray="1 1.600" stroke-width="0.8"/><path d="M8 2.500v11" stroke-width="1.800"/>"#),
    ("border-diag-up", r#"<path d="M2.500 2.500h11v11h-11z" stroke-dasharray="1 1.600" stroke-width="0.8"/><path d="M2.500 13.500l11-11" stroke-width="1.400"/>"#),
    ("border-diag-down", r#"<path d="M2.500 2.500h11v11h-11z" stroke-dasharray="1 1.600" stroke-width="0.8"/><path d="M2.500 2.500l11 11" stroke-width="1.400"/>"#),
    ("pivot-rows", r#"<path d="M2.500 4h11M2.500 8h11M2.500 12h11"/>"#),
    ("pivot-columns", r#"<path d="M4 2.500v11M8 2.500v11M12 2.500v11"/>"#),
    ("pivot-sigma", r#"<path d="M12 4.500V2.500H4L8.500 8 4 13.500h8v-2"/>"#),
    ("misc-trace-precedents", r#"<rect x="9" y="9" width="5" height="4.500"/><circle cx="3" cy="3.500" r="1.100" fill="black"/><path d="M3.600 4.100 8.600 8.900M8.800 5.900v3.200H5.600"/>"#),
    ("misc-trace-dependents", r#"<rect x="2" y="2.500" width="5" height="4.500"/><circle cx="4.500" cy="4.750" r="1.100" fill="black"/><path d="M7.400 7.400 12.600 12.600M12.800 9.600v3.200H9.600"/>"#),
    ("misc-remove-arrows", r#"<path d="M2.500 3.500 8 9M8.200 6v3.200H5"/><path d="M10 10l4 4M14 10l-4 4"/>"#),
    ("misc-show-formulas", r#"<rect x="2.500" y="3.500" width="11" height="9"/><path d="M5.500 6.800h5M5.500 9.200h5"/>"#),
    ("misc-error-checking", r#"<path d="M8 2.500 14 13H2z"/><path d="M8 6.500v3.200"/><circle cx="8" cy="11.300" r="0.600" fill="black"/>"#),
    ("misc-evaluate", r#"<path d="M3 4.500h6M3 8h4M3 11.500h2"/><path d="M9.500 9.500l2 2 2.500-4"/>"#),
    ("misc-watch", r#"<rect x="2.500" y="8.500" width="11" height="5"/><circle cx="5.500" cy="4.500" r="2"/><circle cx="10.500" cy="4.500" r="2"/><path d="M7.500 4.500h1"/>"#),
    ("misc-calc-now", r#"<rect x="2.500" y="2" width="8" height="12" rx="1"/><path d="M4.500 4.500h4v2h-4z"/><path d="M12.500 6v6M11.100 10.600l1.400 1.400 1.400-1.400"/>"#),
    ("misc-calc-sheet", r#"<rect x="2.500" y="2.500" width="11" height="11"/><path d="M2.500 6.200h11M6.200 2.500v11"/><path d="M8.500 9.800h3M10 8.300v3"/>"#),
    ("page-number", r#"<path d="M4 2h8v12H4z"/><path d="M6.300 6.500h1.200v4M9 6.500h1.200v4M5.800 8h4.800M5.800 9.400h4.800" stroke-width="0.9"/>"#),
    ("page-count", r#"<path d="M5.500 2h7v10h-7z"/><path d="M3.500 4.500v9.500h7"/><path d="M7.500 5.500h3M7.500 8h3"/>"#),
    ("page-date", r#"<rect x="2.500" y="3.500" width="11" height="10" rx="1"/><path d="M2.500 6.500h11M5.500 2v3M10.500 2v3"/><path d="M5 9h1.500M7.500 9H9M10 9h1.500M5 11.200h1.500M7.500 11.200H9"/>"#),
    ("page-font", r#"<path d="M3.500 13 8 3l4.500 10M5.300 9.500h5.400"/>"#),
    ("condfmt-new", r#"<rect x="2.5" y="5.5" width="8" height="8"/><path d="M2.5 8.2h8M2.5 10.8h8M6 5.5v8"/><path d="M12.5 1.2v5M10 3.7h5"/>"#),
    ("condfmt-clear", r#"<rect x="2.5" y="2.5" width="8" height="8"/><path d="M2.5 5.2h8M2.5 7.8h8M6 2.5v8"/><path d="M10.2 10.2l4.3 4.3M14.5 10.2l-4.3 4.3" stroke-width="1.4"/>"#),
    ("condfmt-manage", r#"<rect x="2.5" y="2.5" width="11" height="11"/><path d="M2.5 6.2h11M2.5 9.8h11"/><path d="M4.4 4.4h1.2M4.4 8h1.2M4.4 11.6h1.2" stroke-width="1.6"/><path d="M7.5 4.4h4M7.5 8h4M7.5 11.6h4" stroke-width="0.9"/>"#),
    ("filters-clear", r#"<path d="M1.500 2.500h9L7.300 6.800v4.700l-2.600 1.300V6.800z"/><path d="M10.500 9.500l4 4M14.500 9.500l-4 4"/>"#),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        // Turned text of charts is drawn as an image made on the fly.
        if let Some(svg) = crate::chart_view::text_asset(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        let Some(name) = path.strip_prefix("icons/").and_then(|p| p.strip_suffix(".svg")) else {
            return Ok(None);
        };
        Ok(ICONS.iter().find(|(n, _)| *n == name).map(|(_, body)| {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
            );
            Cow::Owned(svg.into_bytes())
        }))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(n, _)| SharedString::from(format!("icons/{n}.svg")))
            .collect())
    }
}

pub fn icon_path(name: &str) -> SharedString {
    SharedString::from(format!("icons/{name}.svg"))
}
