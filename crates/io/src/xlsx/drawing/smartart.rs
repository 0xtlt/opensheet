//! SmartArt graphics. A graphic is a `xdr:graphicFrame` of the drawing
//! part that names five parts of its own (`xl/diagrams/`):
//!
//! * `data`: the outline (points and the connections that make them a
//!   tree), and on its root which layout, style and colours it has;
//! * `layout`, `quickStyle`, `colors`: the definitions of those three;
//! * `drawing`: the shapes the graphic was last laid out as, which is what
//!   an application shows until the graphic is edited.
//!
//! Written: the outline, the shapes as the engine lays them out, and
//! definitions that carry the `uniqueId` of Office's own (so that Office
//! knows which of its layouts, styles and colour variants the graphic
//! has). The definitions themselves are short ones written for this
//! project with the layout algorithms of the format; they are not
//! Office's, which cannot be copied here. The presentation points Office
//! adds to the outline are left out: it makes them again.
//!
//! Read: the outline, and the layout, style and colours by their
//! `uniqueId`. A graphic whose layout is not one the engine draws is kept
//! as its parts and shown through the shapes of its `drawing` part.

use super::*;
use opensheet_engine::smartart::{ColorScheme, Foreign, Layout, Node, Piece, QuickStyle, SmartArt};
use opensheet_engine::theme::Theme as DocumentTheme;

const NS_DIAGRAM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const NS_DSP: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
const REL_DRAWING: &str = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

/// The parts of a graphic: what its relationship is called, where it
/// goes and its content type, in the order of `Foreign::parts`.
const PARTS: [(&str, &str, &str); 5] = [
    (
        "diagramData",
        "data",
        "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml",
    ),
    (
        "diagramLayout",
        "layout",
        "application/vnd.openxmlformats-officedocument.drawingml.diagramLayout+xml",
    ),
    (
        "diagramQuickStyle",
        "quickStyle",
        "application/vnd.openxmlformats-officedocument.drawingml.diagramStyle+xml",
    ),
    (
        "diagramColors",
        "colors",
        "application/vnd.openxmlformats-officedocument.drawingml.diagramColors+xml",
    ),
    ("diagramDrawing", "drawing", "application/vnd.ms-office.drawingml.diagramDrawing+xml"),
];

// ---- writing -------------------------------------------------------------------

/// An identifier of the outline: unique in its part, the same each time
/// the same workbook is written.
fn model_id(kind: u32, index: usize) -> String {
    format!("{{4F53{kind:04X}-0000-4000-8000-{index:012X}}}")
}

/// `<xdr:graphicFrame>`: the object of the anchor. `first` is the
/// relationship to the data part; layout, style and colours follow it.
pub(super) fn push_frame(x: &mut String, drawing: &Drawing, id: usize, first: usize) {
    x.push_str("<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr>");
    push_identity(x, drawing, id);
    w!(
        x,
        "<xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"{NS_DIAGRAM}\"><dgm:relIds xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:r=\"{NS_DOC_REL}\" r:dm=\"rId{first}\" r:lo=\"rId{}\" r:qs=\"rId{}\" r:cs=\"rId{}\"/></a:graphicData></a:graphic></xdr:graphicFrame>",
        first + 1,
        first + 2,
        first + 3
    );
}

fn push_empty_text(x: &mut String) {
    x.push_str("<dgm:t><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\"/></a:p></dgm:t>");
}

/// The data part: the root, a point for each entry, and for each entry
/// the connection to its parent with the two transition points Office
/// hangs on it.
fn data_xml(art: &SmartArt, drawing_rel: usize) -> String {
    fn walk(nodes: &[Node], parent: &str, next: &mut usize, points: &mut String, connections: &mut String) {
        for (order, node) in nodes.iter().enumerate() {
            let index = *next;
            *next += 1;
            let id = model_id(1, index);
            w!(points, "<dgm:pt modelId=\"{id}\">");
            if node.text.is_empty() {
                points.push_str("<dgm:prSet phldrT=\"[Text]\" phldr=\"1\"/><dgm:spPr/>");
                push_empty_text(points);
            } else {
                points.push_str("<dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/>");
                for paragraph in node.text.split('\n') {
                    points.push_str("<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>");
                    push_text(points, paragraph);
                    points.push_str("</a:t></a:r></a:p>");
                }
                points.push_str("</dgm:t>");
            }
            points.push_str("</dgm:pt>");
            let connection = model_id(2, index);
            for (kind, name) in [(3, "parTrans"), (4, "sibTrans")] {
                w!(
                    points,
                    "<dgm:pt modelId=\"{}\" type=\"{name}\" cxnId=\"{connection}\"><dgm:prSet/><dgm:spPr/>",
                    model_id(kind, index)
                );
                push_empty_text(points);
                points.push_str("</dgm:pt>");
            }
            w!(
                connections,
                "<dgm:cxn modelId=\"{connection}\" srcId=\"{parent}\" destId=\"{id}\" srcOrd=\"{order}\" destOrd=\"0\" parTransId=\"{}\" sibTransId=\"{}\"/>",
                model_id(3, index),
                model_id(4, index)
            );
            walk(&node.children, &id, next, points, connections);
        }
    }
    let root = model_id(0, 0);
    let mut x = format!("{XML_DECL}<dgm:dataModel xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:a=\"{NS_DRAWING}\"><dgm:ptLst>");
    w!(
        x,
        "<dgm:pt modelId=\"{root}\" type=\"doc\"><dgm:prSet loTypeId=\"{}\" loCatId=\"{}\" qsTypeId=\"{}\" qsCatId=\"simple\" csTypeId=\"{}\" csCatId=\"{}\"/><dgm:spPr/>",
        art.layout.unique_id(),
        art.layout.category().ooxml_name(),
        art.style.unique_id(),
        art.colors.unique_id(),
        art.colors.category()
    );
    push_empty_text(&mut x);
    x.push_str("</dgm:pt>");
    let (mut points, mut connections) = (String::new(), String::new());
    walk(&art.nodes, &root, &mut 0, &mut points, &mut connections);
    x.push_str(&points);
    x.push_str("</dgm:ptLst><dgm:cxnLst>");
    x.push_str(&connections);
    w!(
        x,
        "</dgm:cxnLst><dgm:bg/><dgm:whole/><dgm:extLst><a:ext uri=\"http://schemas.microsoft.com/office/drawing/2008/diagram\"><dsp:dataModelExt xmlns:dsp=\"{NS_DSP}\" relId=\"rId{drawing_rel}\" minVer=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\"/></a:ext></dgm:extLst></dgm:dataModel>"
    );
    x
}

/// A node of a layout that holds the text of an entry in a shape.
fn text_node(name: &str, shape: &str, label: &str, shows: &str) -> String {
    format!(
        "<dgm:layoutNode name=\"{name}\" styleLbl=\"{label}\"><dgm:varLst><dgm:bulletEnabled val=\"1\"/></dgm:varLst><dgm:alg type=\"tx\"/><dgm:shape type=\"{shape}\" r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf axis=\"{shows}\" ptType=\"node\"/><dgm:constrLst><dgm:constr type=\"lMarg\" refType=\"primFontSz\" fact=\"0.3\"/><dgm:constr type=\"rMarg\" refType=\"primFontSz\" fact=\"0.3\"/><dgm:constr type=\"tMarg\" refType=\"primFontSz\" fact=\"0.3\"/><dgm:constr type=\"bMarg\" refType=\"primFontSz\" fact=\"0.3\"/></dgm:constrLst><dgm:ruleLst><dgm:rule type=\"primFontSz\" val=\"5\" fact=\"NaN\" max=\"NaN\"/></dgm:ruleLst></dgm:layoutNode>"
    )
}

/// What stands between two entries: room (`sp`) or an arrow (`conn`).
fn between(arrow: bool) -> String {
    let body = if arrow {
        "<dgm:layoutNode name=\"sibTrans\" styleLbl=\"sibTrans2D1\"><dgm:alg type=\"conn\"><dgm:param type=\"begPts\" val=\"auto\"/><dgm:param type=\"endPts\" val=\"auto\"/></dgm:alg><dgm:shape type=\"conn\" r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf axis=\"self\"/><dgm:constrLst><dgm:constr type=\"h\" refType=\"w\" fact=\"0.62\"/><dgm:constr type=\"connDist\"/></dgm:constrLst><dgm:ruleLst/></dgm:layoutNode>"
    } else {
        "<dgm:layoutNode name=\"sibTrans\"><dgm:alg type=\"sp\"/><dgm:shape r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf/><dgm:constrLst/><dgm:ruleLst/></dgm:layoutNode>"
    };
    format!("<dgm:forEach name=\"sibTransForEach\" axis=\"followSib\" ptType=\"sibTrans\" cnt=\"1\">{body}</dgm:forEach>")
}

/// The layout part: a definition written for this project that lays the
/// entries out with the algorithm Office's own layout of that
/// `uniqueId` uses.
fn layout_xml(layout: Layout) -> String {
    // The algorithm of the whole graphic, its constraints, and what each
    // entry becomes.
    let per_node = |shape: &str, label: &str, shows: &str, arrow: Option<bool>| {
        format!(
            "<dgm:forEach name=\"nodesForEach\" axis=\"ch\" ptType=\"node\">{}{}</dgm:forEach>",
            text_node("node", shape, label, shows),
            arrow.map(between).unwrap_or_default()
        )
    };
    let sized = |height: &str, gap: &str| {
        format!(
            "<dgm:constrLst><dgm:constr type=\"w\" for=\"ch\" forName=\"node\" refType=\"w\"/><dgm:constr type=\"h\" for=\"ch\" forName=\"node\" refType=\"w\" refFor=\"ch\" refForName=\"node\" fact=\"{height}\"/><dgm:constr type=\"w\" for=\"ch\" forName=\"sibTrans\" refType=\"w\" refFor=\"ch\" refForName=\"node\" fact=\"{gap}\"/><dgm:constr type=\"sp\" refType=\"w\" refFor=\"ch\" refForName=\"sibTrans\"/><dgm:constr type=\"primFontSz\" for=\"ch\" forName=\"node\" op=\"equ\" val=\"65\"/></dgm:constrLst><dgm:ruleLst/>"
        )
    };
    let (priority, algorithm, body) = match layout {
        Layout::BlockList => (
            400,
            "<dgm:alg type=\"snake\"><dgm:param type=\"grDir\" val=\"tL\"/><dgm:param type=\"flowDir\" val=\"row\"/><dgm:param type=\"contDir\" val=\"sameDir\"/><dgm:param type=\"off\" val=\"ctr\"/></dgm:alg>".to_string(),
            sized("0.6", "0.1") + &per_node("rect", "node1", "desOrSelf", Some(false)),
        ),
        Layout::Process => (
            1000,
            "<dgm:alg type=\"lin\"/>".to_string(),
            sized("0.6", "0.4") + &per_node("roundRect", "node1", "desOrSelf", Some(true)),
        ),
        Layout::Chevron => (
            3000,
            "<dgm:alg type=\"lin\"/>".to_string(),
            sized("0.4", "-0.1") + &per_node("chevron", "node1", "desOrSelf", Some(false)),
        ),
        Layout::Cycle => (
            1000,
            "<dgm:alg type=\"cycle\"><dgm:param type=\"stAng\" val=\"0\"/><dgm:param type=\"spanAng\" val=\"360\"/></dgm:alg>".to_string(),
            "<dgm:constrLst><dgm:constr type=\"w\" for=\"ch\" ptType=\"node\" refType=\"w\"/><dgm:constr type=\"w\" for=\"ch\" ptType=\"sibTrans\" refType=\"w\" refFor=\"ch\" refPtType=\"node\" fact=\"0.27\"/><dgm:constr type=\"sibSp\" refType=\"w\" refFor=\"ch\" refPtType=\"node\" fact=\"0.4\"/><dgm:constr type=\"primFontSz\" for=\"ch\" ptType=\"node\" op=\"equ\" val=\"65\"/></dgm:constrLst><dgm:ruleLst/>".to_string()
                + &per_node("ellipse", "node1", "desOrSelf", Some(true)),
        ),
        Layout::Venn => (
            1000,
            "<dgm:alg type=\"cycle\"><dgm:param type=\"stAng\" val=\"0\"/><dgm:param type=\"spanAng\" val=\"360\"/></dgm:alg>".to_string(),
            "<dgm:constrLst><dgm:constr type=\"w\" for=\"ch\" ptType=\"node\" refType=\"w\"/><dgm:constr type=\"h\" for=\"ch\" ptType=\"node\" refType=\"w\" refFor=\"ch\" refPtType=\"node\"/><dgm:constr type=\"sibSp\" refType=\"w\" refFor=\"ch\" refPtType=\"node\" fact=\"-0.38\"/><dgm:constr type=\"primFontSz\" for=\"ch\" ptType=\"node\" op=\"equ\" val=\"65\"/></dgm:constrLst><dgm:ruleLst/>".to_string()
                + &per_node("ellipse", "vennNode1", "desOrSelf", None),
        ),
        Layout::Pyramid => (
            1000,
            "<dgm:alg type=\"pyra\"><dgm:param type=\"linDir\" val=\"fromB\"/><dgm:param type=\"pyraAcctPos\" val=\"aft\"/><dgm:param type=\"pyraAcctTxMar\" val=\"step\"/></dgm:alg>".to_string(),
            "<dgm:constrLst><dgm:constr type=\"primFontSz\" for=\"ch\" ptType=\"node\" op=\"equ\" val=\"65\"/></dgm:constrLst><dgm:ruleLst/>".to_string()
                + &per_node("trapezoid", "node1", "desOrSelf", None),
        ),
        Layout::BulletList => (
            3000,
            "<dgm:alg type=\"lin\"><dgm:param type=\"linDir\" val=\"fromT\"/><dgm:param type=\"vertAlign\" val=\"mid\"/></dgm:alg>".to_string(),
            format!(
                "<dgm:constrLst><dgm:constr type=\"w\" for=\"ch\" forName=\"parentText\" refType=\"w\"/><dgm:constr type=\"h\" for=\"ch\" forName=\"parentText\" refType=\"primFontSz\" refFor=\"ch\" refForName=\"parentText\" fact=\"0.52\"/><dgm:constr type=\"w\" for=\"ch\" forName=\"childText\" refType=\"w\"/><dgm:constr type=\"h\" for=\"ch\" forName=\"childText\" refType=\"primFontSz\" refFor=\"ch\" refForName=\"parentText\" fact=\"0.46\"/><dgm:constr type=\"primFontSz\" for=\"ch\" forName=\"parentText\" op=\"equ\" val=\"65\"/><dgm:constr type=\"secFontSz\" for=\"ch\" forName=\"childText\" refType=\"primFontSz\" refFor=\"ch\" refForName=\"parentText\" op=\"equ\" fact=\"0.78\"/></dgm:constrLst><dgm:ruleLst/><dgm:forEach name=\"nodesForEach\" axis=\"ch\" ptType=\"node\">{}<dgm:choose name=\"details\"><dgm:if name=\"hasDetails\" axis=\"ch\" ptType=\"node\" func=\"cnt\" op=\"gte\" val=\"1\"><dgm:layoutNode name=\"childText\" styleLbl=\"revTx\"><dgm:varLst><dgm:bulletEnabled val=\"1\"/></dgm:varLst><dgm:alg type=\"tx\"><dgm:param type=\"stBulletLvl\" val=\"1\"/><dgm:param type=\"parTxLTRAlign\" val=\"l\"/></dgm:alg><dgm:shape type=\"rect\" r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf axis=\"des\" ptType=\"node\"/><dgm:constrLst/><dgm:ruleLst><dgm:rule type=\"h\" val=\"INF\" fact=\"NaN\" max=\"NaN\"/></dgm:ruleLst></dgm:layoutNode></dgm:if><dgm:else name=\"noDetails\"/></dgm:choose>{}</dgm:forEach>",
                text_node("parentText", "roundRect", "node1", "self"),
                between(false)
            ),
        ),
        Layout::OrgChart => (
            1000,
            "<dgm:alg type=\"hierChild\"><dgm:param type=\"linDir\" val=\"fromL\"/></dgm:alg>".to_string(),
            format!(
                "<dgm:constrLst><dgm:constr type=\"w\" for=\"des\" forName=\"rootText\" refType=\"w\" fact=\"10\"/><dgm:constr type=\"h\" for=\"des\" forName=\"rootText\" refType=\"w\" refFor=\"des\" refForName=\"rootText\" fact=\"0.5\"/><dgm:constr type=\"sibSp\" refType=\"w\" refFor=\"des\" refForName=\"rootText\" fact=\"0.25\"/><dgm:constr type=\"sp\" for=\"des\" forName=\"hierRoot\" refType=\"h\" refFor=\"des\" refForName=\"rootText\" fact=\"0.5\"/><dgm:constr type=\"primFontSz\" for=\"des\" forName=\"rootText\" op=\"equ\" val=\"65\"/></dgm:constrLst><dgm:ruleLst/><dgm:forEach name=\"entry\" axis=\"ch\" ptType=\"node\"><dgm:layoutNode name=\"hierRoot\"><dgm:alg type=\"hierRoot\"><dgm:param type=\"hierAlign\" val=\"tCtrCh\"/></dgm:alg><dgm:shape r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf/><dgm:constrLst/><dgm:ruleLst/>{}<dgm:layoutNode name=\"hierChild\"><dgm:alg type=\"hierChild\"><dgm:param type=\"linDir\" val=\"fromL\"/></dgm:alg><dgm:shape r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf/><dgm:constrLst/><dgm:ruleLst/><dgm:forEach name=\"reports\" axis=\"ch\" ptType=\"node\"><dgm:forEach name=\"line\" axis=\"precedSib\" ptType=\"parTrans\" st=\"-1\" cnt=\"1\"><dgm:layoutNode name=\"parTrans\" styleLbl=\"parChTrans1D2\"><dgm:alg type=\"conn\"><dgm:param type=\"connRout\" val=\"bend\"/><dgm:param type=\"dim\" val=\"1D\"/><dgm:param type=\"endSty\" val=\"noArr\"/><dgm:param type=\"begPts\" val=\"bCtr\"/><dgm:param type=\"endPts\" val=\"tCtr\"/></dgm:alg><dgm:shape type=\"conn\" r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf axis=\"self\"/><dgm:constrLst/><dgm:ruleLst/></dgm:layoutNode></dgm:forEach><dgm:forEach ref=\"entry\"/></dgm:forEach></dgm:layoutNode></dgm:layoutNode></dgm:forEach>",
                text_node("rootText", "rect", "node0", "self")
            ),
        ),
    };
    format!(
        "{XML_DECL}<dgm:layoutDef xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:a=\"{NS_DRAWING}\" uniqueId=\"{}\"><dgm:title val=\"\"/><dgm:desc val=\"\"/><dgm:catLst><dgm:cat type=\"{}\" pri=\"{priority}\"/></dgm:catLst><dgm:layoutNode name=\"diagram\"><dgm:varLst><dgm:dir/><dgm:resizeHandles val=\"exact\"/></dgm:varLst>{algorithm}<dgm:shape xmlns:r=\"{NS_DOC_REL}\" r:blip=\"\"><dgm:adjLst/></dgm:shape><dgm:presOf/>{}</dgm:layoutNode></dgm:layoutDef>",
        layout.unique_id(),
        layout.category().ooxml_name(),
        body.replace("<dgm:shape ", &format!("<dgm:shape xmlns:r=\"{NS_DOC_REL}\" "))
    )
}

/// The labels the layouts above give their shapes.
const LABELS: [&str; 7] = ["node0", "node1", "vennNode1", "sibTrans2D1", "parChTrans1D2", "revTx", "bgShp"];

/// The style part: for each label, which of the theme's lines, fills and
/// fonts its shapes take.
fn style_xml(style: QuickStyle) -> String {
    let (line, fill, effect) = match style {
        QuickStyle::Simple => (2, 1, 0),
        QuickStyle::WhiteOutline => (3, 1, 0),
        QuickStyle::SubtleEffect => (0, 2, 0),
        QuickStyle::ModerateEffect => (0, 2, 2),
        QuickStyle::IntenseEffect => (0, 3, 3),
    };
    let scene = "<dgm:scene3d><a:camera prst=\"orthographicFront\"/><a:lightRig rig=\"threePt\" dir=\"t\"/></dgm:scene3d>";
    let black = "<a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/>";
    let mut x = format!(
        "{XML_DECL}<dgm:styleDef xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:a=\"{NS_DRAWING}\" uniqueId=\"{}\"><dgm:title val=\"\"/><dgm:desc val=\"\"/><dgm:catLst><dgm:cat type=\"simple\" pri=\"{}\"/></dgm:catLst>{scene}",
        style.unique_id(),
        10_100 + 100 * QuickStyle::ALL.iter().position(|other| *other == style).unwrap_or(0)
    );
    for label in LABELS {
        // Text without a shape, and the lines of a chart, have no fill.
        let (line, fill, font) = match label {
            "revTx" => (0, 0, "tx1"),
            "parChTrans1D2" => (2, 0, "tx1"),
            _ => (line, fill, "lt1"),
        };
        w!(
            x,
            "<dgm:styleLbl name=\"{label}\">{scene}<dgm:sp3d/><dgm:txPr/><dgm:style><a:lnRef idx=\"{line}\">{black}</a:lnRef><a:fillRef idx=\"{fill}\">{black}</a:fillRef><a:effectRef idx=\"{effect}\">{black}</a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"{font}\"/></a:fontRef></dgm:style></dgm:styleLbl>"
        );
    }
    x.push_str("</dgm:styleDef>");
    x
}

/// A list of theme colours, each letting half of what is under it show
/// (those that say how much themselves keep their own word).
fn half_shown(colors: &str) -> String {
    let mut out = String::new();
    for entry in colors.split("<a:schemeClr ").filter(|entry| !entry.is_empty()) {
        out.push_str("<a:schemeClr ");
        if entry.contains("<a:alpha") {
            out.push_str(entry);
        } else if let Some(open) = entry.strip_suffix("/>").filter(|open| !open.contains('<')) {
            out.push_str(open);
            out.push_str("><a:alpha val=\"50000\"/></a:schemeClr>");
        } else {
            out.push_str(&entry.replace("</a:schemeClr>", "<a:alpha val=\"50000\"/></a:schemeClr>"));
        }
    }
    out
}

/// The colours part: for each label, the colours of the theme its shapes
/// run through.
fn colors_xml(colors: ColorScheme) -> String {
    let scheme = |name: &str| format!("<a:schemeClr val=\"{name}\"/>");
    let tinted = |name: &str, tint: u32| format!("<a:schemeClr val=\"{name}\"><a:tint val=\"{tint}\"/></a:schemeClr>");
    // How shapes are filled and outlined, and how the colours of the list
    // are handed out (`repeat` one after the other, `span` from the first
    // to the last).
    let (method, fills, lines, priority): (&str, String, String, u32) = match colors {
        ColorScheme::Dark(2) => ("repeat", scheme("dk2"), scheme("lt1"), 10_200),
        ColorScheme::Dark(3) => ("repeat", scheme("lt1"), scheme("dk2"), 10_300),
        ColorScheme::Dark(_) => ("repeat", scheme("lt1"), scheme("dk1"), 10_100),
        ColorScheme::Colorful(1) => (
            "repeat",
            (2..=6).map(|n| scheme(&format!("accent{n}"))).collect(),
            scheme("lt1"),
            10_100,
        ),
        ColorScheme::Colorful(n) => (
            "span",
            scheme(&format!("accent{n}")) + &scheme(&format!("accent{}", n + 1)),
            scheme("lt1"),
            10_000 + 100 * n as u32,
        ),
        ColorScheme::Accent { accent, variant } => {
            let name = format!("accent{accent}");
            let priority = 10_000 + 1000 * accent as u32 + 100 * variant as u32;
            match variant {
                1 => ("repeat", scheme("lt1"), scheme(&name), priority),
                3 | 4 => (
                    if variant == 3 { "span" } else { "cycle" },
                    format!("<a:schemeClr val=\"{name}\"><a:shade val=\"80000\"/></a:schemeClr>") + &tinted(&name, 50_000),
                    scheme("lt1"),
                    priority,
                ),
                5 => (
                    "span",
                    format!("<a:schemeClr val=\"{name}\"><a:alpha val=\"90000\"/></a:schemeClr><a:schemeClr val=\"{name}\"><a:alpha val=\"40000\"/></a:schemeClr>"),
                    scheme("lt1"),
                    priority,
                ),
                _ => ("repeat", scheme(&name), scheme("lt1"), priority),
            }
        }
    };
    let base = match colors {
        ColorScheme::Dark(2 | 3) => "dk2".to_string(),
        ColorScheme::Dark(_) => "dk1".to_string(),
        ColorScheme::Colorful(n) => format!("accent{}", n.max(2)),
        ColorScheme::Accent { accent, .. } => format!("accent{accent}"),
    };
    let mut x = format!(
        "{XML_DECL}<dgm:colorsDef xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:a=\"{NS_DRAWING}\" uniqueId=\"{}\"><dgm:title val=\"\"/><dgm:desc val=\"\"/><dgm:catLst><dgm:cat type=\"{}\" pri=\"{priority}\"/></dgm:catLst>",
        colors.unique_id(),
        colors.category()
    );
    for label in LABELS {
        let (method, fills, lines) = match label {
            "sibTrans2D1" => ("repeat", tinted(&base, 60_000), tinted(&base, 60_000)),
            "parChTrans1D2" => ("repeat", scheme(&base), format!("<a:schemeClr val=\"{base}\"><a:shade val=\"60000\"/></a:schemeClr>")),
            "revTx" | "bgShp" => (
                "repeat",
                "<a:schemeClr val=\"lt1\"><a:alpha val=\"0\"/></a:schemeClr>".to_string(),
                "<a:schemeClr val=\"dk1\"><a:alpha val=\"0\"/></a:schemeClr>".to_string(),
            ),
            "vennNode1" => (method, half_shown(&fills), lines.clone()),
            _ => (method, fills.clone(), lines.clone()),
        };
        w!(
            x,
            "<dgm:styleLbl name=\"{label}\"><dgm:fillClrLst meth=\"{method}\">{fills}</dgm:fillClrLst><dgm:linClrLst meth=\"repeat\">{lines}</dgm:linClrLst><dgm:effectClrLst/><dgm:txLinClrLst/><dgm:txFillClrLst/><dgm:txEffectClrLst/></dgm:styleLbl>"
        );
    }
    x.push_str("</dgm:colorsDef>");
    x
}

/// The drawing part: the shapes of the graphic as they are laid out.
fn cache_xml(pieces: &[Piece]) -> String {
    let mut x = format!(
        "{XML_DECL}<dsp:drawing xmlns:dgm=\"{NS_DIAGRAM}\" xmlns:dsp=\"{NS_DSP}\" xmlns:a=\"{NS_DRAWING}\"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id=\"0\" name=\"\"/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>"
    );
    let black = "<a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/>";
    for (index, piece) in pieces.iter().enumerate() {
        let shape = &piece.shape;
        w!(
            x,
            "<dsp:sp modelId=\"{}\"><dsp:nvSpPr><dsp:cNvPr id=\"0\" name=\"\"/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm",
            model_id(5, index)
        );
        let turn = ((piece.rotation as f64).rem_euclid(360.0) * ANGLE_UNITS).round() as i64;
        if turn != 0 {
            w!(x, " rot=\"{turn}\"");
        }
        if piece.flip_h {
            x.push_str(" flipH=\"1\"");
        }
        let (width, height) = (emu(piece.width as f64), emu(piece.height as f64));
        w!(
            x,
            "><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm>",
            emu(piece.x as f64),
            emu(piece.y as f64)
        );
        push_geometry(&mut x, shape);
        match shape.fill {
            Some(color) => {
                let alpha = (piece.opacity < 1.0).then(|| (piece.opacity.clamp(0.0, 1.0) as f64 * 100_000.0).round() as u32);
                x.push_str("<a:solidFill>");
                push_drawing_color(&mut x, color, alpha);
                x.push_str("</a:solidFill>");
            }
            None => x.push_str("<a:noFill/>"),
        }
        push_outline(&mut x, shape.outline.as_ref());
        w!(
            x,
            "<a:effectLst/></dsp:spPr><dsp:style><a:lnRef idx=\"2\">{black}</a:lnRef><a:fillRef idx=\"1\">{black}</a:fillRef><a:effectRef idx=\"0\">{black}</a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef></dsp:style>"
        );
        if piece.node.is_some() {
            // The text, in the box it is laid out in.
            let [left, top, right, bottom] = shape.text_rect(piece.width, piece.height);
            let size = shape.font.size as u32 * 50;
            w!(
                x,
                "<dsp:txBody><a:bodyPr spcFirstLastPara=\"0\" vert=\"horz\" wrap=\"square\" lIns=\"{0}\" tIns=\"{0}\" rIns=\"{0}\" bIns=\"{0}\" numCol=\"1\" spcCol=\"1270\" anchor=\"{1}\" anchorCtr=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/>",
                size as u64 * 127 * 3 / 10,
                shape.anchor.ooxml_name()
            );
            for paragraph in shape.text.split('\n') {
                w!(
                    x,
                    "<a:p><a:pPr marL=\"0\" lvl=\"0\" indent=\"0\" algn=\"{}\" defTabSz=\"{}\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPct val=\"0\"/></a:spcBef><a:spcAft><a:spcPct val=\"35000\"/></a:spcAft><a:buNone/></a:pPr>",
                    shape.align.ooxml_name(),
                    size as u64 * 127 * 35 / 10
                );
                if paragraph.is_empty() {
                    w!(x, "<a:endParaRPr lang=\"en-US\" sz=\"{size}\" kern=\"1200\"/>");
                } else {
                    w!(x, "<a:r><a:rPr lang=\"en-US\" sz=\"{size}\" kern=\"1200\">");
                    if let Some(color) = shape.font.color {
                        push_solid_fill(&mut x, color);
                    }
                    x.push_str("</a:rPr><a:t>");
                    push_text(&mut x, paragraph);
                    x.push_str("</a:t></a:r>");
                }
                x.push_str("</a:p>");
            }
            let at = |fraction: f32, side: f32, from: f32| emu((from + fraction * side) as f64);
            w!(
                x,
                "</dsp:txBody><dsp:txXfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></dsp:txXfrm>",
                at(left, piece.width, piece.x),
                at(top, piece.height, piece.y),
                emu(((right - left) * piece.width) as f64),
                emu(((bottom - top) * piece.height) as f64)
            );
        }
        x.push_str("</dsp:sp>");
    }
    x.push_str("</dsp:spTree></dsp:drawing>");
    x
}

/// Put `relId` of the data part of a graphic kept from a file in line
/// with the relationship its drawing part gets this time.
fn with_drawing_rel(data: &str, drawing_rel: usize) -> String {
    let Some(at) = data.find("dataModelExt") else {
        return data.to_string();
    };
    let Some(start) = data[at..].find("relId=\"").map(|found| at + found + 7) else {
        return data.to_string();
    };
    let end = data[start..].find('"').map_or(data.len(), |found| start + found);
    format!("{}rId{drawing_rel}{}", &data[..start], &data[end..])
}

/// Add the five parts of a graphic to the package; returns what the
/// drawing part relates to, in order: (relationship type, target). The
/// relationship of the first is `first`, the others follow it. `number`
/// counts the graphics of the workbook.
pub(super) fn add_parts(art: &SmartArt, theme: &DocumentTheme, size: (f64, f64), number: usize, first: usize, extras: &mut Extras) -> Vec<(String, String)> {
    let drawing_rel = first + 4;
    let contents: [String; 5] = match &art.foreign {
        Some(foreign) => {
            let mut parts = foreign.parts.clone();
            parts[0] = with_drawing_rel(&parts[0], drawing_rel);
            parts
        }
        None => [
            data_xml(art, drawing_rel),
            layout_xml(art.layout),
            style_xml(art.style),
            colors_xml(art.colors),
            cache_xml(&art.pieces(theme, size.0 as f32, size.1 as f32)),
        ],
    };
    let mut related = Vec::new();
    for ((kind, file, content_type), xml) in PARTS.into_iter().zip(contents) {
        let name = format!("{file}{number}.xml");
        extras.add_part(format!("xl/diagrams/{name}"), Some(content_type), xml);
        let kind = if kind == "diagramDrawing" {
            REL_DRAWING.to_string()
        } else {
            format!("{NS_DOC_REL}/{kind}")
        };
        related.push((kind, format!("../diagrams/{name}")));
    }
    related
}

// ---- reading -------------------------------------------------------------------

/// The relationships of a `<xdr:graphicFrame>` that holds a graphic: its
/// data, layout, style and colours parts. `None` for any other frame (a
/// chart). Consumes the frame.
pub(super) fn parse_frame<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, object: &mut RawObject) -> Option<[String; 4]> {
    let mut found = None;
    each_child(r, tag, |r, t| match t.name {
        "nvGraphicFramePr" => {
            object.parse_identity(r, t);
        }
        "graphic" => each_child(r, t, |r, data| {
            each_child(r, data, |r, ids| {
                if ids.name == "relIds" {
                    let id = |name: &str| ids.attr_local(name).map(|id| id.into_owned()).unwrap_or_default();
                    found = Some([id("dm"), id("lo"), id("qs"), id("cs")]);
                }
                r.skip(ids);
            });
        }),
        _ => r.skip(t),
    });
    found.filter(|ids| !ids[0].is_empty())
}

/// What the data part says: the outline, the layout, style and colours
/// named on its root, and the relationship to the drawing part.
struct Data {
    nodes: Vec<Node>,
    layout: String,
    style: String,
    colors: String,
    drawing_rel: Option<String>,
}

fn parse_data(xml: &str) -> Data {
    /// A point: its kind, and its text when it is an entry.
    struct Point {
        id: String,
        entry: bool,
        text: String,
    }
    let mut data = Data {
        nodes: Vec::new(),
        layout: String::new(),
        style: String::new(),
        colors: String::new(),
        drawing_rel: None,
    };
    let mut points: Vec<Point> = Vec::new();
    let mut root = String::new();
    // (parent, child, place among its siblings)
    let mut links: Vec<(String, String, u32)> = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "dataModel" | "ptLst" | "cxnLst" | "extLst" | "ext" => {}
                "pt" => {
                    let id = t.attr("modelId").unwrap_or_default().into_owned();
                    let kind = t.attr("type").map(|kind| kind.into_owned()).unwrap_or_else(|| "node".to_string());
                    let mut text = String::new();
                    each_child(&mut r, &t, |r, c| match c.name {
                        "prSet" => {
                            if kind == "doc" {
                                let named = |name: &str| c.attr(name).unwrap_or_default().into_owned();
                                (data.layout, data.style, data.colors) = (named("loTypeId"), named("qsTypeId"), named("csTypeId"));
                            }
                            r.skip(c);
                        }
                        "t" => {
                            let mut paragraphs = 0;
                            each_child(r, c, |r, p| {
                                if p.name != "p" {
                                    r.skip(p);
                                    return;
                                }
                                if paragraphs > 0 {
                                    text.push('\n');
                                }
                                paragraphs += 1;
                                each_child(r, p, |r, run| match run.name {
                                    "r" | "fld" => each_child(r, run, |r, part| match part.name {
                                        "t" => text.push_str(&printable(&r.text(part), false)),
                                        _ => r.skip(part),
                                    }),
                                    _ => r.skip(run),
                                });
                            });
                        }
                        _ => r.skip(c),
                    });
                    if kind == "doc" {
                        root = id.clone();
                    }
                    points.push(Point {
                        id,
                        entry: matches!(kind.as_str(), "node" | "asst"),
                        text,
                    });
                }
                "cxn" => {
                    // A connection without a type makes its end a child
                    // of its start.
                    if t.attr("type").is_none_or(|kind| kind == "parOf") {
                        let end = |name: &str| t.attr(name).unwrap_or_default().into_owned();
                        links.push((end("srcId"), end("destId"), t.attr_u32("srcOrd").unwrap_or(0)));
                    }
                    r.skip(&t);
                }
                "dataModelExt" => {
                    data.drawing_rel = t.attr("relId").map(|id| id.into_owned());
                    r.skip(&t);
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    links.sort_by_key(|(_, _, order)| *order);
    /// The entries under a point; `depth` guards against outlines that
    /// go round in circles.
    fn children(parent: &str, points: &[Point], links: &[(String, String, u32)], depth: usize) -> Vec<Node> {
        if depth > 32 {
            return Vec::new();
        }
        links
            .iter()
            .filter(|(from, _, _)| from == parent)
            .filter_map(|(_, to, _)| points.iter().find(|point| point.id == *to && point.entry))
            .map(|point| Node {
                text: point.text.clone(),
                children: children(&point.id, points, links, depth + 1),
            })
            .collect()
    }
    data.nodes = children(&root, &points, &links, 0);
    data
}

/// The `uniqueId` a definition part gives itself.
fn unique_id(xml: &str) -> Option<String> {
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => return t.attr("uniqueId").map(|id| id.into_owned()),
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The shapes of a drawing part, with their boxes in pixels.
fn parse_cache(xml: &str, theme: &Theme) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "drawing" | "spTree" => {}
                "sp" => {
                    // Once for where the shape is, once for what it is.
                    let mut placed = r.fork();
                    let (mut at, mut size) = ((0.0, 0.0), (0.0, 0.0));
                    each_child(&mut placed, &t, |r, c| match c.name {
                        "spPr" => each_child(r, c, |r, xfrm| {
                            if xfrm.name != "xfrm" {
                                r.skip(xfrm);
                                return;
                            }
                            each_child(r, xfrm, |r, part| {
                                let pair = |a: &str, b: &str| (part.attr_f64(a).map_or(0.0, px), part.attr_f64(b).map_or(0.0, px));
                                match part.name {
                                    "off" => at = pair("x", "y"),
                                    "ext" => size = pair("cx", "cy"),
                                    _ => {}
                                }
                                r.skip(part);
                            });
                        }),
                        _ => r.skip(c),
                    });
                    let object = parse_shape(&mut r, &t, theme);
                    if let RawKind::Shape(shape) = object.kind {
                        pieces.push(Piece {
                            node: (!shape.text.is_empty()).then_some(pieces.len()),
                            shape,
                            x: at.0 as f32,
                            y: at.1 as f32,
                            width: size.0 as f32,
                            height: size.1 as f32,
                            rotation: object.rotation,
                            flip_h: object.flip_h,
                            opacity: 1.0,
                        });
                    }
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    pieces
}

/// Read the graphic a frame names. `rels` are the relationships of the
/// drawing part. `None` when its data part is missing.
pub(super) fn load<R: Read + Seek>(pkg: &mut Package<R>, rels: &[Rel], ids: &[String; 4], theme: &Theme) -> Option<SmartArt> {
    let mut part = |id: &str| {
        let target = rels.iter().find(|rel| rel.id == id).map(|rel| rel.target.clone())?;
        pkg.read(&target).ok().flatten()
    };
    let data_xml = part(&ids[0])?;
    let definitions = [part(&ids[1]), part(&ids[2]), part(&ids[3])];
    let data = parse_data(&data_xml);
    // The definitions say what they are; the root of the outline says it
    // too, for a file that lost one.
    let named = |index: usize, fallback: &str| definitions[index].as_deref().and_then(unique_id).unwrap_or_else(|| fallback.to_string());
    let layout = Layout::from_unique_id(&named(0, &data.layout));
    let mut art = SmartArt {
        layout: layout.unwrap_or(Layout::BlockList),
        nodes: data.nodes,
        colors: ColorScheme::from_unique_id(&named(2, &data.colors)).unwrap_or_default(),
        style: QuickStyle::from_unique_id(&named(1, &data.style)).unwrap_or_default(),
        rtl: false,
        foreign: None,
    };
    if layout.is_none() {
        // A layout that is not drawn here: the graphic as its file has it.
        let drawing = data
            .drawing_rel
            .as_deref()
            .and_then(&mut part)
            .or_else(|| {
                let target = rels.iter().find(|rel| rel.kind == "diagramDrawing").map(|rel| rel.target.clone())?;
                pkg.read(&target).ok().flatten()
            })
            .unwrap_or_default();
        let [layout_xml, style_xml, colors_xml] = definitions.map(Option::unwrap_or_default);
        art.foreign = Some(Arc::new(Foreign {
            pieces: parse_cache(&drawing, theme),
            // The frame of the graphic, once it is placed on its sheet.
            size: (1.0, 1.0),
            parts: [data_xml, layout_xml, style_xml, colors_xml, drawing],
        }));
    }
    Some(art)
}
