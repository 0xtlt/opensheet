//! SmartArt: a graphic drawn from a small outline of text.
//!
//! A graphic is a tree of text nodes ([`Node`]), a layout that says what
//! shapes the nodes become ([`Layout`]), colours taken from the accents of
//! the theme ([`ColorScheme`]) and a style ([`QuickStyle`]). Nothing else
//! is stored: [`SmartArt::pieces`] lays the graphic out again in whatever
//! frame it is given, as a list of ordinary shapes ([`Piece`]) that a
//! screen, a printer and a file writer can all follow.
//!
//! The outline is edited as the Text Pane shows it: a list of lines, each
//! at a level ([`SmartArt::outline`]). A line is a child of the nearest
//! line above it that is one level higher, so promoting, demoting, adding
//! and removing lines are edits of that list.
//!
//! A graphic read from a file whose layout is not one of these keeps the
//! shapes its file had laid out ([`Foreign`]) and is shown through them.
//!
//! Sizes of text are estimated from the number of characters (the engine
//! has no fonts): the text of a graphic is as large as lets every node of
//! a level fit its shape, as in Excel.

use crate::drawing::{Outline, Shape, ShapePreset, ShapeTool, TextAlign, TextAnchor};
use crate::locale::Locale;
use crate::style::{Color, ThemeSlot};
use crate::theme::Theme;
use crate::validation::localized;
use std::sync::Arc;

/// One entry of the outline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(text: &str) -> Node {
        Node {
            text: text.to_string(),
            children: Vec::new(),
        }
    }
}

/// The gallery's categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    List,
    Process,
    Cycle,
    Hierarchy,
    Relationship,
    Pyramid,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::List,
        Category::Process,
        Category::Cycle,
        Category::Hierarchy,
        Category::Relationship,
        Category::Pyramid,
    ];

    /// The name files give the category (`loCatId`).
    pub fn ooxml_name(self) -> &'static str {
        match self {
            Category::List => "list",
            Category::Process => "process",
            Category::Cycle => "cycle",
            Category::Hierarchy => "hierarchy",
            Category::Relationship => "relationship",
            Category::Pyramid => "pyramid",
        }
    }
}

/// The layouts that are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layout {
    /// Basic Block List: blocks in rows.
    BlockList,
    /// Vertical Bullet List: a bar for each entry, its details under it.
    BulletList,
    /// Basic Process: boxes in a row, an arrow between two.
    Process,
    /// Basic Chevron Process: a row of chevrons.
    Chevron,
    /// Basic Cycle: circles around a circle, an arrow between two.
    Cycle,
    /// Organization Chart: a tree, parents over their children.
    OrgChart,
    /// Basic Venn: circles that overlap.
    Venn,
    /// Basic Pyramid: levels, the first on top.
    Pyramid,
}

impl Layout {
    pub const ALL: [Layout; 8] = [
        Layout::BlockList,
        Layout::BulletList,
        Layout::Process,
        Layout::Chevron,
        Layout::Cycle,
        Layout::OrgChart,
        Layout::Venn,
        Layout::Pyramid,
    ];

    pub fn category(self) -> Category {
        match self {
            Layout::BlockList | Layout::BulletList => Category::List,
            Layout::Process | Layout::Chevron => Category::Process,
            Layout::Cycle => Category::Cycle,
            Layout::OrgChart => Category::Hierarchy,
            Layout::Venn => Category::Relationship,
            Layout::Pyramid => Category::Pyramid,
        }
    }

    /// What Office calls the layout in files (the end of its `uniqueId`).
    pub fn ooxml_name(self) -> &'static str {
        match self {
            Layout::BlockList => "default",
            Layout::BulletList => "vList2",
            Layout::Process => "process1",
            Layout::Chevron => "chevron1",
            Layout::Cycle => "cycle2",
            Layout::OrgChart => "orgChart1",
            Layout::Venn => "venn1",
            Layout::Pyramid => "pyramid1",
        }
    }

    /// The identifier of Office's own definition of the layout.
    pub fn unique_id(self) -> String {
        format!("urn:microsoft.com/office/officeart/2005/8/layout/{}", self.ooxml_name())
    }

    pub fn from_unique_id(id: &str) -> Option<Layout> {
        let name = id.rsplit('/').next()?;
        Layout::ALL.into_iter().find(|layout| layout.ooxml_name() == name)
    }

    /// The outline a new graphic starts with: empty entries.
    pub fn sample(self) -> Vec<Node> {
        let empty = |count: usize| vec![Node::default(); count];
        match self {
            Layout::BlockList | Layout::Cycle => empty(5),
            Layout::Process | Layout::Chevron | Layout::Venn | Layout::Pyramid => empty(3),
            Layout::BulletList => vec![
                Node {
                    text: String::new(),
                    children: empty(1),
                };
                2
            ],
            Layout::OrgChart => vec![Node {
                text: String::new(),
                children: empty(3),
            }],
        }
    }
}

// ---- colours -------------------------------------------------------------

/// A colour between two others (`t` from 0 to 1).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    // At its ends the range is the colours themselves, which may follow
    // the theme.
    if t == 0.0 {
        return a;
    } else if t == 1.0 {
        return b;
    }
    let channel = |shift: u32| {
        let (x, y) = (((a.value() >> shift) & 0xFF) as f32, ((b.value() >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    Color(channel(16) | channel(8) | channel(0))
}

/// The choices of Change Colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    /// Primary theme colours: 1 "Dark 1 Outline", 2 "Dark 2 Fill",
    /// 3 "Dark 2 Outline".
    Dark(u8),
    /// 1 "Colorful - Accent Colors"; 2 to 5 "Colorful Range - Accent
    /// Colors 2 to 3" … "5 to 6".
    Colorful(u8),
    /// For each accent (1 to 6): 1 "Colored Outline", 2 "Colored Fill",
    /// 3 "Gradient Range", 4 "Gradient Loop", 5 "Transparent Gradient
    /// Range".
    Accent { accent: u8, variant: u8 },
}

impl Default for ColorScheme {
    /// "Colored Fill - Accent 1", what a new graphic has.
    fn default() -> ColorScheme {
        ColorScheme::Accent { accent: 1, variant: 2 }
    }
}

/// How one shape of a graphic is painted.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Paint {
    fill: Color,
    line: Color,
    text: Color,
    /// The fill lets what is under it show (1: not at all).
    opacity: f32,
    /// The colour the scheme is about: arrows and connectors derive
    /// theirs from it.
    base: Color,
    /// The shape is filled with the colour (not only outlined in it).
    filled: bool,
}

impl ColorScheme {
    /// Every choice, in the order of the Change Colors gallery.
    pub fn all() -> Vec<ColorScheme> {
        let mut out = vec![ColorScheme::Dark(1), ColorScheme::Dark(3), ColorScheme::Dark(2)];
        out.extend((1..=5).map(ColorScheme::Colorful));
        for accent in 1..=6 {
            out.extend((1..=5).map(|variant| ColorScheme::Accent { accent, variant }));
        }
        out
    }

    /// What Office calls the variant in files (the end of its `uniqueId`).
    pub fn ooxml_name(self) -> String {
        match self {
            ColorScheme::Dark(n) => format!("accent0_{}", n.clamp(1, 3)),
            ColorScheme::Colorful(n) => format!("colorful{}", n.clamp(1, 5)),
            ColorScheme::Accent { accent, variant } => format!("accent{}_{}", accent.clamp(1, 6), variant.clamp(1, 5)),
        }
    }

    pub fn unique_id(self) -> String {
        format!("urn:microsoft.com/office/officeart/2005/8/colors/{}", self.ooxml_name())
    }

    /// The category files give the variant (`csCatId`).
    pub fn category(self) -> String {
        match self {
            ColorScheme::Dark(_) => "mainScheme".to_string(),
            ColorScheme::Colorful(_) => "colorful".to_string(),
            ColorScheme::Accent { accent, .. } => format!("accent{}", accent.clamp(1, 6)),
        }
    }

    pub fn from_unique_id(id: &str) -> Option<ColorScheme> {
        let name = id.rsplit('/').next()?;
        ColorScheme::all().into_iter().find(|scheme| scheme.ooxml_name() == name)
    }

    /// The paint of the `index`th of `count` shapes.
    fn paint(self, theme: &Theme, index: usize, count: usize) -> Paint {
        let (dark_1, light_1, dark_2) = (theme.color(ThemeSlot::Dark1, 0.0), theme.color(ThemeSlot::Light1, 0.0), theme.color(ThemeSlot::Dark2, 0.0));
        let accent = |number: u8| theme.accent(number.clamp(1, 6) as usize, 0.0);
        let along = if count > 1 { index as f32 / (count - 1) as f32 } else { 0.0 };
        let outlined = |color: Color| Paint {
            fill: light_1,
            line: color,
            text: dark_1,
            opacity: 1.0,
            base: color,
            filled: false,
        };
        let filled = |color: Color| Paint {
            fill: color,
            line: light_1,
            text: light_1,
            opacity: 1.0,
            base: color,
            filled: true,
        };
        match self {
            ColorScheme::Dark(2) => filled(dark_2),
            ColorScheme::Dark(3) => outlined(dark_2),
            ColorScheme::Dark(_) => outlined(dark_1),
            ColorScheme::Colorful(1) => filled(accent(2 + (index % 5) as u8)),
            ColorScheme::Colorful(n) => filled(mix(accent(n), accent(n + 1), along)),
            ColorScheme::Accent { accent: a, variant } => {
                let color = accent(a);
                match variant {
                    1 => outlined(color),
                    3 => filled(mix(mix(color, dark_1, 0.2), mix(color, light_1, 0.5), along)),
                    4 => filled(mix(mix(color, dark_1, 0.2), mix(color, light_1, 0.5), 1.0 - (2.0 * along - 1.0).abs())),
                    5 => Paint {
                        opacity: 0.9 - 0.5 * along,
                        ..filled(color)
                    },
                    _ => filled(color),
                }
            }
        }
    }
}

/// The flat SmartArt Styles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QuickStyle {
    /// "Simple Fill".
    #[default]
    Simple,
    WhiteOutline,
    SubtleEffect,
    ModerateEffect,
    IntenseEffect,
}

impl QuickStyle {
    pub const ALL: [QuickStyle; 5] = [
        QuickStyle::Simple,
        QuickStyle::WhiteOutline,
        QuickStyle::SubtleEffect,
        QuickStyle::ModerateEffect,
        QuickStyle::IntenseEffect,
    ];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            QuickStyle::Simple => "simple1",
            QuickStyle::WhiteOutline => "simple2",
            QuickStyle::SubtleEffect => "simple3",
            QuickStyle::ModerateEffect => "simple4",
            QuickStyle::IntenseEffect => "simple5",
        }
    }

    pub fn unique_id(self) -> String {
        format!("urn:microsoft.com/office/officeart/2005/8/quickstyle/{}", self.ooxml_name())
    }

    pub fn from_unique_id(id: &str) -> Option<QuickStyle> {
        let name = id.rsplit('/').next()?;
        QuickStyle::ALL.into_iter().find(|style| style.ooxml_name() == name)
    }

    /// The fill, the outline and the colour of the text of a shape
    /// painted with `paint` in this style.
    fn look(self, paint: Paint, theme: &Theme) -> (Color, Option<Outline>, Color) {
        let (dark_1, light_1) = (theme.color(ThemeSlot::Dark1, 0.0), theme.color(ThemeSlot::Light1, 0.0));
        let line = |color: Color, width: f32| Some(Outline::solid(color, width));
        match self {
            QuickStyle::Simple => (paint.fill, line(paint.line, 1.0), paint.text),
            QuickStyle::WhiteOutline => (paint.fill, line(paint.line, 3.0), paint.text),
            QuickStyle::SubtleEffect if paint.filled => (mix(paint.fill, light_1, 0.6), line(paint.base, 0.5), dark_1),
            QuickStyle::SubtleEffect => (paint.fill, line(paint.line, 0.5), paint.text),
            QuickStyle::ModerateEffect if paint.filled => (paint.fill, None, paint.text),
            QuickStyle::ModerateEffect => (paint.fill, line(paint.line, 1.5), paint.text),
            QuickStyle::IntenseEffect if paint.filled => (mix(paint.fill, dark_1, 0.15), None, paint.text),
            QuickStyle::IntenseEffect => (paint.fill, line(paint.line, 2.25), paint.text),
        }
    }
}

// ---- the graphic ---------------------------------------------------------

/// One shape of a graphic that was laid out: a shape in a box, in pixels
/// from the top-left corner of the frame of the graphic.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub shape: Shape,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Degrees, clockwise, about the middle of the box.
    pub rotation: f32,
    pub flip_h: bool,
    /// How much of the fill hides what is under it (0 to 1).
    pub opacity: f32,
    /// The line of the outline the shape shows; `None` for arrows and
    /// connectors.
    pub node: Option<usize>,
}

/// What a file held of a graphic whose layout is not drawn here: the
/// shapes it had laid out, and its parts as they were, to write them back.
#[derive(Clone, Debug, PartialEq)]
pub struct Foreign {
    pub pieces: Vec<Piece>,
    /// The size the pieces were laid out for.
    pub size: (f32, f32),
    /// The data, layout, style, colours and drawing parts.
    pub parts: [String; 5],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmartArt {
    pub layout: Layout,
    pub nodes: Vec<Node>,
    pub colors: ColorScheme,
    pub style: QuickStyle,
    /// "Right to Left": the graphic runs the other way.
    pub rtl: bool,
    /// The graphic as its file had it, when its layout is not one of
    /// `Layout`: it is shown from this until it is given a layout.
    pub foreign: Option<Arc<Foreign>>,
    /// The graphic as a file had it, when its layout is one of `Layout`:
    /// as long as nothing was changed since, it goes back to a file with
    /// the parts it came with.
    pub origin: Option<Arc<Origin>>,
}

/// What a file held of a graphic that is drawn here: its parts as they
/// were, and what was read from them.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    /// The data, layout, style, colours and drawing parts.
    pub parts: [String; 5],
    pub layout: Layout,
    pub nodes: Vec<Node>,
    pub colors: ColorScheme,
    pub style: QuickStyle,
    pub rtl: bool,
}

/// The name Excel gives a graphic, before its number.
pub fn name(locale: &Locale) -> &'static str {
    localized(locale, ["Diagram", "Diagramme", "Diagrama", "Diagramm", "Diagramma"])
}

/// What an empty entry shows in its shape on screen.
pub fn placeholder(locale: &Locale) -> &'static str {
    localized(locale, ["[Text]", "[Texte]", "[Texto]", "[Text]", "[Testo]"])
}

/// The size of a new graphic, in pixels at 100 %: 6 by 4 inches.
pub const DEFAULT_SIZE: (f32, f32) = (576.0, 384.0);

/// A line of the outline: how deep it is and what it says.
pub type Line = (usize, String);

fn flatten(nodes: &[Node], level: usize, out: &mut Vec<Line>) {
    for node in nodes {
        out.push((level, node.text.clone()));
        flatten(&node.children, level + 1, out);
    }
}

/// The tree of a list of lines. Levels are brought in order first: the
/// first line is at the top, and no line is more than one level deeper
/// than the one before it.
fn tree(lines: &[Line]) -> Vec<Node> {
    fn build(lines: &[(usize, &str)], at: &mut usize, level: usize) -> Vec<Node> {
        let mut out = Vec::new();
        while *at < lines.len() && lines[*at].0 >= level {
            let mut node = Node::new(lines[*at].1);
            *at += 1;
            node.children = build(lines, at, level + 1);
            out.push(node);
        }
        out
    }
    let mut previous = None;
    let mut settled = Vec::with_capacity(lines.len());
    for (level, text) in lines {
        let level = match previous {
            None => 0,
            Some(before) => (*level).min(before + 1),
        };
        previous = Some(level);
        settled.push((level, text.as_str()));
    }
    build(&settled, &mut 0, 0)
}

impl SmartArt {
    /// A new graphic as Excel inserts it.
    pub fn new(layout: Layout) -> SmartArt {
        SmartArt {
            layout,
            nodes: layout.sample(),
            colors: ColorScheme::default(),
            style: QuickStyle::default(),
            rtl: false,
            foreign: None,
            origin: None,
        }
    }

    /// The parts the graphic came with from its file, while it is as it
    /// was read: a file writer puts them back as they were. `None` for a
    /// graphic made or changed here.
    pub fn kept_parts(&self) -> Option<&[String; 5]> {
        if let Some(foreign) = &self.foreign {
            return Some(&foreign.parts);
        }
        let origin = self.origin.as_ref()?;
        let same = origin.layout == self.layout
            && origin.nodes == self.nodes
            && origin.colors == self.colors
            && origin.style == self.style
            && origin.rtl == self.rtl;
        same.then_some(&origin.parts)
    }

    /// How many shapes the graphic is laid out as (whatever its frame).
    pub fn piece_count(&self) -> usize {
        self.pieces(&Theme::default(), DEFAULT_SIZE.0, DEFAULT_SIZE.1).len()
    }

    /// How many objects the graphic is in a file: its frame alone while
    /// it keeps the parts it came with; else its frame, and the group of
    /// its shapes written beside it for applications that would lay it
    /// out their own way.
    pub fn object_count(&self) -> usize {
        match self.kept_parts() {
            Some(_) => 1,
            None => 2 + self.piece_count(),
        }
    }

    /// The outline as the Text Pane shows it.
    pub fn outline(&self) -> Vec<Line> {
        let mut out = Vec::new();
        flatten(&self.nodes, 0, &mut out);
        out
    }

    /// Replace the outline (levels are brought in order).
    pub fn set_outline(&mut self, lines: &[Line]) {
        self.nodes = tree(lines);
    }

    /// One past the last line of the entry at `index` and of everything
    /// under it.
    fn end_of(lines: &[Line], index: usize) -> usize {
        let level = lines[index].0;
        lines[index + 1..]
            .iter()
            .position(|line| line.0 <= level)
            .map_or(lines.len(), |offset| index + 1 + offset)
    }

    /// Edit the outline as a list of lines; `false` (and nothing changes)
    /// when `edit` says the change cannot be made.
    fn edit(&mut self, edit: impl FnOnce(&mut Vec<Line>) -> bool) -> bool {
        let mut lines = self.outline();
        if !edit(&mut lines) {
            return false;
        }
        self.set_outline(&lines);
        true
    }

    pub fn set_text(&mut self, index: usize, text: &str) -> bool {
        self.edit(|lines| match lines.get_mut(index) {
            Some(line) if line.1 != text => {
                line.1 = text.to_string();
                true
            }
            _ => false,
        })
    }

    /// Return in the Text Pane: a new line right under line `index`, at
    /// its level. Returns the index of the new line.
    pub fn add_line_after(&mut self, index: usize) -> Option<usize> {
        self.edit(|lines| match lines.get(index) {
            Some((level, _)) => {
                let level = *level;
                lines.insert(index + 1, (level, String::new()));
                true
            }
            None => false,
        })
        .then_some(index + 1)
    }

    /// "Add Shape After": a new entry at the level of entry `index`,
    /// after it and everything under it. An empty outline gets its first
    /// entry whatever `index` is.
    pub fn add_after(&mut self, index: usize) -> Option<usize> {
        let mut added = None;
        self.edit(|lines| {
            if lines.is_empty() {
                lines.push((0, String::new()));
                added = Some(0);
            } else if index < lines.len() {
                let end = SmartArt::end_of(lines, index);
                lines.insert(end, (lines[index].0, String::new()));
                added = Some(end);
            }
            added.is_some()
        });
        added
    }

    /// "Add Shape Before": a new entry at the level of entry `index`,
    /// before it.
    pub fn add_before(&mut self, index: usize) -> Option<usize> {
        self.edit(|lines| match lines.get(index) {
            Some((level, _)) => {
                let level = *level;
                lines.insert(index, (level, String::new()));
                true
            }
            None => false,
        })
        .then_some(index)
    }

    /// "Add Shape Above": a new entry takes the place of entry `index`,
    /// which becomes its child.
    pub fn add_above(&mut self, index: usize) -> Option<usize> {
        self.edit(|lines| {
            if index >= lines.len() {
                return false;
            }
            let level = lines[index].0;
            let end = SmartArt::end_of(lines, index);
            for line in &mut lines[index..end] {
                line.0 += 1;
            }
            lines.insert(index, (level, String::new()));
            true
        })
        .then_some(index)
    }

    /// "Add Shape Below": a new entry under entry `index`, after the
    /// children it has.
    pub fn add_below(&mut self, index: usize) -> Option<usize> {
        let mut added = None;
        self.edit(|lines| {
            if index >= lines.len() {
                return false;
            }
            let end = SmartArt::end_of(lines, index);
            lines.insert(end, (lines[index].0 + 1, String::new()));
            added = Some(end);
            true
        });
        added
    }

    /// Remove line `index`; what was under it goes to the line above.
    pub fn remove(&mut self, index: usize) -> bool {
        self.edit(|lines| {
            if index >= lines.len() {
                return false;
            }
            lines.remove(index);
            true
        })
    }

    /// One level up, with everything under it. Not for the top level.
    pub fn promote(&mut self, index: usize) -> bool {
        self.edit(|lines| {
            if index >= lines.len() || lines[index].0 == 0 {
                return false;
            }
            let end = SmartArt::end_of(lines, index);
            for line in &mut lines[index..end] {
                line.0 -= 1;
            }
            true
        })
    }

    /// One level down, with everything under it: it becomes the last
    /// child of the entry before it. Not for an entry that has none
    /// before it at its level.
    pub fn demote(&mut self, index: usize) -> bool {
        self.edit(|lines| {
            if index == 0 || index >= lines.len() || lines[index - 1].0 < lines[index].0 {
                return false;
            }
            let end = SmartArt::end_of(lines, index);
            for line in &mut lines[index..end] {
                line.0 += 1;
            }
            true
        })
    }

    /// The entry before entry `index` under the same parent.
    fn previous_sibling(lines: &[Line], index: usize) -> Option<usize> {
        let level = lines.get(index)?.0;
        lines[..index]
            .iter()
            .rposition(|line| line.0 <= level)
            .filter(|before| lines[*before].0 == level)
    }

    /// Move an entry, with everything under it, before the entry that
    /// precedes it under their parent (`up`) or after the one that
    /// follows it. Returns where the entry is afterwards.
    pub fn move_entry(&mut self, index: usize, up: bool) -> Option<usize> {
        let mut moved = None;
        self.edit(|lines| {
            if index >= lines.len() {
                return false;
            }
            let end = SmartArt::end_of(lines, index);
            let level = lines[index].0;
            if up {
                let Some(before) = SmartArt::previous_sibling(lines, index) else {
                    return false;
                };
                lines[before..end].rotate_left(index - before);
                moved = Some(before);
            } else {
                if lines.get(end).is_none_or(|next| next.0 != level) {
                    return false;
                }
                let next_end = SmartArt::end_of(lines, end);
                lines[index..next_end].rotate_left(end - index);
                moved = Some(index + next_end - end);
            }
            true
        });
        moved
    }

    /// "Reset Graphic": the colours and the style of a new graphic.
    pub fn reset(&mut self) {
        self.colors = ColorScheme::default();
        self.style = QuickStyle::default();
    }

    /// Give the graphic another layout; its text stays. A graphic shown
    /// through the shapes of its file is drawn here from then on.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.foreign = None;
    }

    /// The graphic laid out in a frame `width` by `height` pixels: its
    /// shapes, bottom-most first.
    pub fn pieces(&self, theme: &Theme, width: f32, height: f32) -> Vec<Piece> {
        let (width, height) = (width.max(1.0), height.max(1.0));
        if let Some(foreign) = &self.foreign {
            let (sx, sy) = (width / foreign.size.0.max(1.0), height / foreign.size.1.max(1.0));
            return foreign
                .pieces
                .iter()
                .cloned()
                .map(|mut piece| {
                    piece.x *= sx;
                    piece.width *= sx;
                    piece.y *= sy;
                    piece.height *= sy;
                    piece
                })
                .collect();
        }
        let mut drawn = Composer {
            art: self,
            theme,
            pieces: Vec::new(),
        };
        match self.layout {
            Layout::BlockList => drawn.block_list(width, height),
            Layout::BulletList => drawn.bullet_list(width, height),
            Layout::Process => drawn.process(width, height),
            Layout::Chevron => drawn.chevrons(width, height),
            Layout::Cycle => drawn.cycle(width, height),
            Layout::OrgChart => drawn.org_chart(width, height),
            Layout::Venn => drawn.venn(width, height),
            Layout::Pyramid => drawn.pyramid(width, height),
        }
        let mut pieces = drawn.pieces;
        fit_text(&mut pieces);
        if self.rtl {
            for piece in &mut pieces {
                piece.x = width - piece.x - piece.width;
                // What points along the flow points the other way.
                if piece.node.is_none() || piece.shape.preset == ShapePreset::Chevron {
                    piece.flip_h = !piece.flip_h;
                    piece.rotation = (360.0 - piece.rotation).rem_euclid(360.0);
                }
            }
        }
        pieces
    }
}

// ---- text ----------------------------------------------------------------

/// The text a shape shows for an entry: its own, then what is under it as
/// bullets.
fn text_with_bullets(node: &Node) -> String {
    fn bullets(nodes: &[Node], depth: usize, out: &mut String) {
        for node in nodes {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
            out.push_str("• ");
            out.push_str(&node.text);
            bullets(&node.children, depth + 1, out);
        }
    }
    let mut out = node.text.clone();
    bullets(&node.children, 0, &mut out);
    out
}

/// Width of a character and height of a line, as fractions of the size of
/// the text: what an average text in a sans-serif font takes.
const CHAR_WIDTH: f32 = 0.55;
const LINE_HEIGHT: f32 = 1.2;

/// The largest and the smallest text of a graphic, in points.
const MAX_TEXT_PT: f32 = 40.0;
const MIN_TEXT_PT: f32 = 5.0;

/// True when a text fits a box (pixels) at a size (pixels), broken at
/// spaces.
fn fits(text: &str, width: f32, height: f32, size: f32) -> bool {
    // The margins of the text in its shape (a tenth of an inch on each
    // side), and a little more as the text grows.
    let room = width - 19.2 - 0.3 * size;
    let per_row = (room / (CHAR_WIDTH * size)).floor();
    if per_row < 1.0 {
        return false;
    }
    let mut rows = 0.0f32;
    for paragraph in text.split('\n') {
        let mut used = 0.0f32;
        rows += 1.0;
        for word in paragraph.split(' ') {
            let length = word.chars().count() as f32;
            if length > per_row {
                return false;
            }
            let needed = if used > 0.0 { used + 1.0 + length } else { length };
            if needed > per_row {
                rows += 1.0;
                used = length;
            } else {
                used = needed;
            }
        }
    }
    rows * LINE_HEIGHT * size <= height - 9.6 - 0.2 * size
}

/// Give the text of every piece the largest size at which the texts of
/// all pieces of its kind fit their shapes: entries have one size, the
/// details under them another.
fn fit_text(pieces: &mut [Piece]) {
    for details in [false, true] {
        let of_kind = |piece: &Piece| piece.node.is_some() && (piece.shape.fill.is_none() && piece.shape.outline.is_none()) == details;
        let fits_all = |points: f32| {
            let size = points * 96.0 / 72.0;
            pieces.iter().filter(|piece| of_kind(piece)).all(|piece| {
                let [left, top, right, bottom] = piece.shape.text_rect(piece.width, piece.height);
                // An empty entry shows a placeholder of six characters.
                let text = if piece.shape.text.is_empty() { "[Text]" } else { &piece.shape.text };
                fits(text, (right - left) * piece.width, (bottom - top) * piece.height, size)
            })
        };
        let mut points = MAX_TEXT_PT;
        while points > MIN_TEXT_PT && !fits_all(points) {
            points -= if points > 20.0 { 2.0 } else { 1.0 };
        }
        for piece in pieces.iter_mut().filter(|piece| of_kind(piece)) {
            piece.shape.font.set_size_pt(points);
        }
    }
}

// ---- layouts -------------------------------------------------------------

struct Composer<'a> {
    art: &'a SmartArt,
    theme: &'a Theme,
    pieces: Vec<Piece>,
}

impl Composer<'_> {
    /// The shape of an entry: the `index`th of `count` shapes of its
    /// kind, showing line `line` of the outline.
    #[allow(clippy::too_many_arguments)]
    fn node(&mut self, preset: ShapePreset, frame: (f32, f32, f32, f32), text: String, line: usize, index: usize, count: usize) -> &mut Piece {
        let paint = self.art.colors.paint(self.theme, index, count);
        let (fill, outline, ink) = self.art.style.look(paint, self.theme);
        let mut shape = ShapeTool::Rectangle.shape();
        shape.preset = preset;
        shape.fill = Some(fill);
        shape.outline = outline;
        shape.font.color = Some(ink);
        // An entry with details under it reads from the top left.
        if text.contains('\n') {
            shape.align = TextAlign::Left;
            shape.anchor = TextAnchor::Top;
        }
        shape.text = text;
        self.pieces.push(Piece {
            shape,
            x: frame.0,
            y: frame.1,
            width: frame.2,
            height: frame.3,
            rotation: 0.0,
            flip_h: false,
            opacity: paint.opacity,
            node: Some(line),
        });
        self.pieces.last_mut().expect("just pushed")
    }

    /// Text without a shape around it: the details under an entry.
    fn details(&mut self, frame: (f32, f32, f32, f32), text: String, line: usize) {
        let mut shape = ShapeTool::Rectangle.shape();
        shape.fill = None;
        shape.outline = None;
        shape.font.color = Some(self.theme.color(ThemeSlot::Dark1, 0.0));
        shape.align = TextAlign::Left;
        shape.anchor = TextAnchor::Top;
        shape.text = text;
        self.pieces.push(Piece {
            shape,
            x: frame.0,
            y: frame.1,
            width: frame.2,
            height: frame.3,
            rotation: 0.0,
            flip_h: false,
            opacity: 1.0,
            node: Some(line),
        });
    }

    /// An arrow between two entries, pointing right before it is turned.
    fn arrow(&mut self, centre: (f32, f32), size: (f32, f32), rotation: f32, index: usize, count: usize) {
        let paint = self.art.colors.paint(self.theme, index, count);
        let mut shape = ShapeTool::RightArrow.shape();
        shape.fill = Some(mix(paint.base, self.theme.color(ThemeSlot::Light1, 0.0), 0.6));
        shape.outline = None;
        self.pieces.push(Piece {
            shape,
            x: centre.0 - size.0 / 2.0,
            y: centre.1 - size.1 / 2.0,
            width: size.0,
            height: size.1,
            rotation,
            flip_h: false,
            opacity: 1.0,
            node: None,
        });
    }

    /// A level or upright connector of an organization chart.
    fn connector(&mut self, from: (f32, f32), to: (f32, f32)) {
        let paint = self.art.colors.paint(self.theme, 0, 1);
        let mut shape = ShapeTool::Line.shape();
        shape.outline = Some(Outline::solid(mix(paint.base, self.theme.color(ThemeSlot::Dark1, 0.0), 0.4), 1.0));
        self.pieces.push(Piece {
            shape,
            x: from.0.min(to.0),
            y: from.1.min(to.1),
            width: (to.0 - from.0).abs(),
            height: (to.1 - from.1).abs(),
            rotation: 0.0,
            flip_h: false,
            opacity: 1.0,
            node: None,
        });
    }

    /// The entries of the top level with the line of the outline each is
    /// on.
    fn top(&self) -> Vec<(usize, &Node)> {
        fn lines(node: &Node) -> usize {
            1 + node.children.iter().map(lines).sum::<usize>()
        }
        let mut at = 0;
        let mut out = Vec::new();
        for node in &self.art.nodes {
            out.push((at, node));
            at += lines(node);
        }
        // `self.art` outlives `self`: the nodes are borrowed from it.
        out
    }

    /// Blocks in rows: as many to a row as suits the proportions of the
    /// frame; a last row that is not full is centred.
    fn block_list(&mut self, width: f32, height: f32) {
        let nodes: Vec<(usize, String)> = self.top().into_iter().map(|(line, node)| (line, text_with_bullets(node))).collect();
        let count = nodes.len();
        if count == 0 {
            return;
        }
        // A block is 0.6 times as high as wide, a tenth of its width apart.
        let (aspect, gap) = (0.6, 0.1);
        let side = |columns: usize| {
            let rows = count.div_ceil(columns);
            let by_width = width / (columns as f32 + (columns - 1) as f32 * gap);
            let by_height = height / (rows as f32 * aspect + (rows - 1) as f32 * gap);
            by_width.min(by_height)
        };
        let columns = ((count as f32 * width / height * aspect).sqrt().ceil() as usize).clamp(1, count);
        let w = side(columns);
        let rows = count.div_ceil(columns);
        let top = (height - (rows as f32 * aspect + (rows - 1) as f32 * gap) * w) / 2.0;
        for (i, (line, text)) in nodes.into_iter().enumerate() {
            let (row, column) = (i / columns, i % columns);
            let in_row = if row + 1 == rows { count - row * columns } else { columns };
            let left = (width - (in_row as f32 + (in_row - 1) as f32 * gap) * w) / 2.0;
            let frame = (left + column as f32 * (1.0 + gap) * w, top + row as f32 * (aspect + gap) * w, w, aspect * w);
            self.node(ShapePreset::Rect, frame, text, line, i, count);
        }
    }

    /// A bar across the frame for each entry, and under it the entries it
    /// has, as bullets.
    fn bullet_list(&mut self, width: f32, height: f32) {
        fn bullets(nodes: &[Node], depth: usize, out: &mut Vec<String>) {
            for node in nodes {
                out.push(format!("{}• {}", "  ".repeat(depth), node.text));
                bullets(&node.children, depth + 1, out);
            }
        }
        let entries: Vec<(usize, String, Vec<String>)> = self
            .top()
            .into_iter()
            .map(|(line, node)| {
                let mut lines = Vec::new();
                bullets(&node.children, 0, &mut lines);
                (line, node.text.clone(), lines)
            })
            .collect();
        let count = entries.len();
        if count == 0 {
            return;
        }
        // A bar is one unit high; a bullet 0.55; bars are 0.12 apart.
        let (bullet, gap) = (0.55, 0.12);
        let units: f32 = entries.iter().map(|(_, _, lines)| 1.0 + lines.len() as f32 * bullet).sum::<f32>() + (count - 1) as f32 * gap;
        let unit = (height / units).min(width * 0.2);
        let mut y = (height - units * unit) / 2.0;
        for (i, (line, text, lines)) in entries.into_iter().enumerate() {
            let bar = self.node(ShapePreset::RoundRect, (0.0, y, width, unit), text, line, i, count);
            bar.shape.align = TextAlign::Left;
            bar.shape.anchor = TextAnchor::Middle;
            y += unit;
            if !lines.is_empty() {
                let room = lines.len() as f32 * bullet * unit;
                self.details((unit * 0.3, y, width - unit * 0.3, room), lines.join("\n"), line + 1);
                y += room;
            }
            y += gap * unit;
        }
    }

    /// Boxes in a row with an arrow between two.
    fn process(&mut self, width: f32, height: f32) {
        let nodes: Vec<(usize, String)> = self.top().into_iter().map(|(line, node)| (line, text_with_bullets(node))).collect();
        let count = nodes.len();
        if count == 0 {
            return;
        }
        // Between two boxes, 0.4 of their width, half of it an arrow.
        let (aspect, between) = (0.6, 0.4);
        let w = (width / (count as f32 + (count - 1) as f32 * between)).min(height / aspect);
        let left = (width - (count as f32 + (count - 1) as f32 * between) * w) / 2.0;
        let middle = height / 2.0;
        for (i, (line, text)) in nodes.into_iter().enumerate() {
            let x = left + i as f32 * (1.0 + between) * w;
            self.node(ShapePreset::RoundRect, (x, middle - aspect * w / 2.0, w, aspect * w), text, line, i, count);
            if i + 1 < count {
                self.arrow((x + w + between * w / 2.0, middle), (between * w * 0.53, w * 0.25), 0.0, i, count);
            }
        }
    }

    /// Chevrons in a row, each reaching into the notch of the next; the
    /// entries under an entry are listed below its chevron.
    fn chevrons(&mut self, width: f32, height: f32) {
        let top: Vec<(usize, String, String)> = self
            .top()
            .into_iter()
            .map(|(line, node)| {
                let details = text_with_bullets(&Node {
                    text: String::new(),
                    children: node.children.clone(),
                });
                (line, node.text.clone(), details.trim_start_matches('\n').to_string())
            })
            .collect();
        let count = top.len();
        if count == 0 {
            return;
        }
        let detailed = top.iter().any(|(_, _, details)| !details.is_empty());
        // Chevrons are 0.4 times as high as wide and overlap by a tenth.
        let (aspect, overlap) = (0.4, 0.1);
        let w = width / (count as f32 - (count - 1) as f32 * overlap);
        let h = (aspect * w).min(if detailed { height * 0.4 } else { height });
        let y = if detailed { 0.0 } else { (height - h) / 2.0 };
        for (i, (line, text, details)) in top.into_iter().enumerate() {
            let x = i as f32 * (1.0 - overlap) * w;
            self.node(ShapePreset::Chevron, (x, y, w, h), text, line, i, count);
            if !details.is_empty() {
                self.details((x + h * 0.25, h + height * 0.03, w * (1.0 - overlap) - h * 0.25, height - h - height * 0.03), details, line + 1);
            }
        }
    }

    /// Circles around a circle, clockwise from the top, with an arrow
    /// from each to the next.
    fn cycle(&mut self, width: f32, height: f32) {
        let nodes: Vec<(usize, String)> = self.top().into_iter().map(|(line, node)| (line, text_with_bullets(node))).collect();
        let count = nodes.len();
        if count == 0 {
            return;
        }
        let side = width.min(height);
        let centre = (width / 2.0, height / 2.0);
        if count == 1 {
            let d = side * 0.6;
            self.node(ShapePreset::Ellipse, (centre.0 - d / 2.0, centre.1 - d / 2.0, d, d), nodes[0].1.clone(), nodes[0].0, 0, 1);
            return;
        }
        // Neighbours are 1.4 diameters apart, centre to centre.
        let step = std::f32::consts::TAU / count as f32;
        let d = side / (1.0 + 1.4 / (step / 2.0).sin());
        let radius = (side - d) / 2.0;
        for (i, (line, text)) in nodes.into_iter().enumerate() {
            let angle = i as f32 * step - std::f32::consts::FRAC_PI_2;
            let (x, y) = (centre.0 + radius * angle.cos(), centre.1 + radius * angle.sin());
            self.node(ShapePreset::Ellipse, (x - d / 2.0, y - d / 2.0, d, d), text, line, i, count);
            // Half way to the next, pointing along the circle.
            let between = angle + step / 2.0;
            let at = (centre.0 + radius * between.cos(), centre.1 + radius * between.sin());
            self.arrow(at, (d * 0.27, d * 0.34), between.to_degrees() + 90.0, i, count);
        }
    }

    /// A tree: each entry over the entries it has, joined by lines.
    fn org_chart(&mut self, width: f32, height: f32) {
        /// A box of the chart: its line in the outline, its column (in
        /// box widths, of its middle) and its row.
        struct Placed {
            line: usize,
            text: String,
            column: f32,
            row: usize,
            parent: Option<usize>,
        }
        /// Place a subtree from column `left` on; returns the columns it
        /// takes. Boxes are one column wide and a quarter apart.
        fn place(node: &Node, line: &mut usize, left: f32, row: usize, parent: Option<usize>, out: &mut Vec<Placed>) -> f32 {
            let me = out.len();
            out.push(Placed {
                line: *line,
                text: node.text.clone(),
                column: 0.0,
                row,
                parent,
            });
            *line += 1;
            let mut taken = 0.0;
            for (i, child) in node.children.iter().enumerate() {
                if i > 0 {
                    taken += 0.25;
                }
                taken += place(child, line, left + taken, row + 1, Some(me), out);
            }
            let span = taken.max(1.0);
            out[me].column = if node.children.is_empty() {
                left + 0.5
            } else {
                // Over the middle of its children.
                let first = out[me + 1].column;
                let last = out.iter().rev().find(|placed| placed.parent == Some(me)).map_or(first, |placed| placed.column);
                (first + last) / 2.0 + (span - taken) / 2.0
            };
            span
        }
        let mut boxes: Vec<Placed> = Vec::new();
        let mut line = 0;
        let mut columns = 0.0;
        for (i, node) in self.art.nodes.iter().enumerate() {
            if i > 0 {
                columns += 0.25;
            }
            columns += place(node, &mut line, columns, 0, None, &mut boxes);
        }
        if boxes.is_empty() {
            return;
        }
        let rows = boxes.iter().map(|placed| placed.row).max().unwrap_or(0) + 1;
        // A box is half as high as wide; rows are half a box apart.
        let (aspect, gap) = (0.5, 0.5);
        let w = (width / columns).min(height / (aspect * (rows as f32 + (rows - 1) as f32 * gap)));
        let h = aspect * w;
        let left = (width - columns * w) / 2.0;
        let top = (height - h * (rows as f32 + (rows - 1) as f32 * gap)) / 2.0;
        let centre = |placed: &Placed| (left + placed.column * w, top + placed.row as f32 * h * (1.0 + gap));
        // Lines first: they lie under the boxes.
        for placed in &boxes {
            let Some(parent) = placed.parent else {
                continue;
            };
            let (x, y) = centre(placed);
            let (px, py) = centre(&boxes[parent]);
            let bend = py + h + h * gap / 2.0;
            self.connector((px, py + h), (px, bend));
            if (px - x).abs() > 0.01 {
                self.connector((px, bend), (x, bend));
            }
            self.connector((x, bend), (x, y));
        }
        let count = boxes.len();
        for (i, placed) in boxes.iter().enumerate() {
            let (x, y) = centre(placed);
            self.node(ShapePreset::Rect, (x - w / 2.0, y, w, h), placed.text.clone(), placed.line, i, count);
        }
    }

    /// Circles that overlap, around the middle of the frame; two lie side
    /// by side.
    fn venn(&mut self, width: f32, height: f32) {
        let nodes: Vec<(usize, String)> = self.top().into_iter().map(|(line, node)| (line, text_with_bullets(node))).collect();
        let count = nodes.len();
        if count == 0 {
            return;
        }
        let centre = (width / 2.0, height / 2.0);
        let step = std::f32::consts::TAU / count as f32;
        // Neighbours are 0.62 diameters apart, centre to centre.
        let reach = if count == 1 { 0.0 } else { 0.31 / (step / 2.0).sin() };
        let d = if count == 2 {
            height.min(width / (1.0 + 2.0 * reach))
        } else {
            width.min(height) / (1.0 + 2.0 * reach)
        };
        for (i, (line, text)) in nodes.into_iter().enumerate() {
            // Two: left and right; more: from the top, clockwise.
            let angle = if count == 2 {
                std::f32::consts::PI * (1.0 + i as f32)
            } else {
                i as f32 * step - std::f32::consts::FRAC_PI_2
            };
            let (x, y) = (centre.0 + reach * d * angle.cos(), centre.1 + reach * d * angle.sin());
            let ink = self.theme.color(ThemeSlot::Dark1, 0.0);
            let circle = self.node(ShapePreset::Ellipse, (x - d / 2.0, y - d / 2.0, d, d), text, line, i, count);
            circle.opacity = circle.opacity.min(0.5);
            // Over one another, what is written must stay readable.
            circle.shape.font.color = Some(ink);
        }
    }

    /// Levels of a triangle, the first on top.
    fn pyramid(&mut self, width: f32, height: f32) {
        let nodes: Vec<(usize, String)> = self.top().into_iter().map(|(line, node)| (line, text_with_bullets(node))).collect();
        let count = nodes.len();
        if count == 0 {
            return;
        }
        let h = height / count as f32;
        for (i, (line, text)) in nodes.into_iter().enumerate() {
            // As wide as the triangle is at its foot.
            let w = width * (i + 1) as f32 / count as f32;
            let frame = ((width - w) / 2.0, i as f32 * h, w, h);
            if i == 0 {
                self.node(ShapePreset::Triangle, frame, text, line, i, count);
            } else {
                let level = self.node(ShapePreset::Trapezoid, frame, text, line, i, count);
                // Its top is as wide as the foot of the level above.
                level.shape.adjust = Some(width / (2.0 * count as f32) / w.min(h).max(1.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(non_snake_case)]
    fn LIGHT_1() -> Color {
        theme().color(ThemeSlot::Light1, 0.0)
    }

    #[allow(non_snake_case)]
    fn DARK_1() -> Color {
        theme().color(ThemeSlot::Dark1, 0.0)
    }

    fn theme() -> Theme {
        Theme::default()
    }

    fn accent(number: usize) -> Color {
        theme().accent(number, 0.0)
    }

    fn outline(art: &SmartArt) -> String {
        art.outline().iter().map(|(level, text)| format!("{}{text}", ">".repeat(*level))).collect::<Vec<_>>().join(" ")
    }

    fn lettered(layout: Layout, lines: &[(usize, &str)]) -> SmartArt {
        let mut art = SmartArt::new(layout);
        art.set_outline(&lines.iter().map(|(level, text)| (*level, text.to_string())).collect::<Vec<_>>());
        art
    }

    #[test]
    fn the_outline_is_a_list_of_lines() {
        let mut art = lettered(Layout::OrgChart, &[(0, "a"), (1, "b"), (2, "c"), (1, "d"), (0, "e")]);
        assert_eq!(outline(&art), "a >b >>c >d e");
        assert_eq!(art.nodes.len(), 2);
        assert_eq!(art.nodes[0].children[0].children[0].text, "c");
        // Levels that skip are brought back, and the first line is on top.
        art.set_outline(&[(3, "x".into()), (5, "y".into()), (1, "z".into())]);
        assert_eq!(outline(&art), "x >y >z");
        // Return adds a line at the level of the one it is typed on; what
        // was under that line is now under the new one.
        let mut art = lettered(Layout::OrgChart, &[(0, "a"), (1, "b"), (0, "c")]);
        assert_eq!(art.add_line_after(0), Some(1));
        assert_eq!(outline(&art), "a  >b c");
        assert!(art.set_text(1, "n") && !art.set_text(1, "n") && !art.set_text(9, "?"));
        // Demote and promote take what is under the entry along.
        assert!(art.demote(1));
        assert_eq!(outline(&art), "a >n >>b c");
        assert!(!art.demote(1) && !art.demote(0) && !art.promote(0));
        assert!(art.promote(1));
        assert_eq!(outline(&art), "a n >b c");
        // Removing a line hands what was under it to the line above.
        assert!(art.remove(1));
        assert_eq!(outline(&art), "a >b c");
        assert!(!art.remove(7));
    }

    #[test]
    fn shapes_are_added_around_an_entry() {
        let mut art = lettered(Layout::OrgChart, &[(0, "a"), (1, "b"), (1, "c"), (0, "d")]);
        // After: at its level, past everything under it.
        assert_eq!(art.add_after(0), Some(3));
        assert_eq!(outline(&art), "a >b >c  d");
        assert_eq!(art.add_before(1), Some(1));
        assert_eq!(outline(&art), "a > >b >c  d");
        // Below: a child, after the children it has.
        assert_eq!(art.add_below(0), Some(4));
        assert_eq!(outline(&art), "a > >b >c >  d");
        // Above: in its place, with the entry under it.
        let mut art = lettered(Layout::OrgChart, &[(0, "a"), (1, "b"), (2, "c"), (1, "d")]);
        assert_eq!(art.add_above(1), Some(1));
        assert_eq!(outline(&art), "a > >>b >>>c >d");
        assert_eq!(art.add_after(99), None);
        // Entries change places with their neighbours, under one parent.
        let mut art = lettered(Layout::OrgChart, &[(0, "a"), (1, "b"), (0, "c"), (1, "d"), (1, "e")]);
        assert_eq!(art.move_entry(2, true), Some(0));
        assert_eq!(outline(&art), "c >d >e a >b");
        assert_eq!(art.move_entry(0, false), Some(2));
        assert_eq!(outline(&art), "a >b c >d >e");
        assert_eq!(art.move_entry(3, false), Some(4));
        assert_eq!(outline(&art), "a >b c >e >d");
        // Not past the first or the last of its parent.
        assert_eq!(art.move_entry(3, true), None);
        assert_eq!(art.move_entry(4, false), None);
        assert_eq!(art.move_entry(2, false), None);
        // An outline emptied gets its first entry back.
        let mut empty = lettered(Layout::Process, &[]);
        assert_eq!(empty.add_after(0), Some(0));
        assert_eq!(empty.nodes.len(), 1);
    }

    fn overlap(a: &Piece, b: &Piece) -> bool {
        a.x < b.x + b.width - 0.01 && b.x < a.x + a.width - 0.01 && a.y < b.y + b.height - 0.01 && b.y < a.y + a.height - 0.01
    }

    #[test]
    fn every_layout_fits_its_frame() {
        for layout in Layout::ALL {
            for count in [1usize, 2, 3, 5, 8] {
                let mut art = SmartArt::new(layout);
                art.set_outline(&(0..count).flat_map(|i| [(0, format!("Entry {i}")), (1, "Detail".to_string())]).collect::<Vec<_>>());
                for (width, height) in [(576.0, 384.0), (200.0, 600.0), (900.0, 120.0)] {
                    let pieces = art.pieces(&theme(), width, height);
                    let shown = pieces.iter().filter(|piece| piece.node.is_some()).count();
                    assert!(shown >= count, "{layout:?} {count}: {shown} shapes");
                    for piece in &pieces {
                        assert!(
                            piece.x >= -0.5 && piece.y >= -0.5 && piece.x + piece.width <= width + 0.5 && piece.y + piece.height <= height + 0.5,
                            "{layout:?} {count} in {width}x{height}: {piece:?}"
                        );
                        assert!(piece.width.is_finite() && piece.height.is_finite() && piece.width >= 0.0 && piece.height >= 0.0);
                        let size = piece.shape.font.size_pt();
                        assert!((MIN_TEXT_PT..=MAX_TEXT_PT).contains(&size), "{layout:?}: {size}");
                    }
                }
            }
            assert_eq!(Layout::from_unique_id(&layout.unique_id()), Some(layout));
            assert!(!layout.sample().is_empty());
        }
        assert_eq!(Layout::from_unique_id("urn:microsoft.com/office/officeart/2005/8/layout/hList1"), None);
    }

    #[test]
    fn layouts_place_their_shapes() {
        // Five blocks in a frame of 6 by 4 inches: rows of three and two,
        // the second centred under the first.
        let blocks = SmartArt::new(Layout::BlockList).pieces(&theme(), 576.0, 384.0);
        assert_eq!(blocks.len(), 5);
        assert!((blocks[0].y - blocks[2].y).abs() < 0.01 && blocks[3].y > blocks[0].y + blocks[0].height);
        assert!((blocks[0].height / blocks[0].width - 0.6).abs() < 1e-3);
        assert!(blocks[3].x > blocks[0].x && blocks[4].x + blocks[4].width < blocks[2].x + blocks[2].width);
        assert!(!blocks.iter().enumerate().any(|(i, a)| blocks[i + 1..].iter().any(|b| overlap(a, b))));
        // A process: three boxes, two arrows between them, in a row.
        let process = SmartArt::new(Layout::Process).pieces(&theme(), 576.0, 384.0);
        assert_eq!(process.iter().filter(|piece| piece.node.is_none()).count(), 2);
        assert_eq!(process[1].shape.preset, ShapePreset::RightArrow);
        assert!(process[0].x + process[0].width < process[1].x && process[1].x + process[1].width < process[2].x);
        // Right to left, the first box is on the right and arrows point left.
        let mut reversed = SmartArt::new(Layout::Process);
        reversed.rtl = true;
        let mirrored = reversed.pieces(&theme(), 576.0, 384.0);
        assert!((mirrored[0].x + mirrored[0].width - (576.0 - process[0].x)).abs() < 0.01);
        assert!(mirrored[1].flip_h && !mirrored[0].flip_h);
        // Chevrons reach into one another.
        let chevrons = SmartArt::new(Layout::Chevron).pieces(&theme(), 600.0, 300.0);
        assert!(chevrons.iter().all(|piece| piece.shape.preset == ShapePreset::Chevron));
        assert!(chevrons[1].x < chevrons[0].x + chevrons[0].width && chevrons[2].x + chevrons[2].width <= 600.01);
        // A cycle starts at the top and runs clockwise, an arrow after
        // each circle.
        let cycle = SmartArt::new(Layout::Cycle).pieces(&theme(), 400.0, 400.0);
        assert_eq!(cycle.len(), 10);
        assert!((cycle[0].x + cycle[0].width / 2.0 - 200.0).abs() < 0.01 && cycle[0].y.abs() < 0.01);
        assert!(cycle[2].x > cycle[0].x && cycle[2].y > cycle[0].y);
        assert!((cycle[1].rotation - 36.0).abs() < 0.01, "{}", cycle[1].rotation);
        let circles: Vec<&Piece> = cycle.iter().filter(|piece| piece.node.is_some()).collect();
        assert!(!circles.iter().enumerate().any(|(i, a)| circles[i + 1..].iter().any(|b| overlap(a, b) && (a.x - b.x).hypot(a.y - b.y) < a.width)));
        // An organization chart: the head over its three reports, joined
        // by lines that lie under the boxes.
        let chart = SmartArt::new(Layout::OrgChart).pieces(&theme(), 576.0, 384.0);
        let boxes: Vec<&Piece> = chart.iter().filter(|piece| piece.node.is_some()).collect();
        assert_eq!(boxes.len(), 4);
        assert!(chart[0].node.is_none() && chart.iter().filter(|piece| piece.node.is_none()).all(|line| line.width == 0.0 || line.height == 0.0));
        assert!((boxes[0].x + boxes[0].width / 2.0 - (boxes[2].x + boxes[2].width / 2.0)).abs() < 0.01);
        assert!(boxes[1].y > boxes[0].y + boxes[0].height && (boxes[1].y - boxes[3].y).abs() < 0.01);
        // Venn circles overlap and let one another show.
        let venn = SmartArt::new(Layout::Venn).pieces(&theme(), 576.0, 384.0);
        assert!(overlap(&venn[0], &venn[1]) && venn.iter().all(|piece| piece.opacity <= 0.5));
        // The levels of a pyramid line up into a triangle.
        let pyramid = SmartArt::new(Layout::Pyramid).pieces(&theme(), 300.0, 300.0);
        assert_eq!(pyramid[0].shape.preset, ShapePreset::Triangle);
        assert_eq!((pyramid[2].x, pyramid[2].width), (0.0, 300.0));
        let inset = pyramid[2].shape.adjust.unwrap() * pyramid[2].height;
        assert!((inset - 50.0).abs() < 0.01 && (pyramid[1].x - 50.0).abs() < 0.01);
        // A bullet list: a bar for each entry, its details under it.
        let list = SmartArt::new(Layout::BulletList).pieces(&theme(), 576.0, 384.0);
        assert_eq!(list.len(), 4);
        assert!(list[1].shape.fill.is_none() && list[1].y >= list[0].y + list[0].height - 0.01);
        assert_eq!(list[1].shape.text, "• ");
        assert_eq!((list[0].node, list[1].node, list[2].node), (Some(0), Some(1), Some(2)));
    }

    #[test]
    fn text_is_as_large_as_fits_every_shape() {
        let mut art = lettered(Layout::Process, &[(0, "One"), (0, "Two"), (0, "Three")]);
        let short = art.pieces(&theme(), 576.0, 384.0)[0].shape.font.size_pt();
        art.set_text(1, "A much longer description of the second step");
        let pieces = art.pieces(&theme(), 576.0, 384.0);
        let long = pieces[0].shape.font.size_pt();
        assert!(long < short, "{long} {short}");
        // One size for every entry.
        assert!(pieces.iter().filter(|piece| piece.node.is_some()).all(|piece| piece.shape.font.size_pt() == long));
        // What is under an entry is listed in its shape.
        art.set_outline(&[(0, "Plan".into()), (1, "Scope".into()), (2, "Budget".into()), (0, "Do".into())]);
        let pieces = art.pieces(&theme(), 576.0, 384.0);
        assert_eq!(pieces[0].shape.text, "Plan\n• Scope\n  • Budget");
        assert_eq!((pieces[0].shape.align, pieces[2].shape.align), (TextAlign::Left, TextAlign::Center));
        assert_eq!((pieces[0].node, pieces[2].node), (Some(0), Some(3)));
        assert!(fits("ab cd", 100.0, 40.0, 16.0) && !fits("abcdefghijklmnop", 60.0, 400.0, 16.0) && !fits("a\nb\nc", 100.0, 40.0, 16.0));
    }

    #[test]
    fn colours_and_styles() {
        assert_eq!(ColorScheme::all().len(), 38);
        for scheme in ColorScheme::all() {
            assert_eq!(ColorScheme::from_unique_id(&scheme.unique_id()), Some(scheme), "{scheme:?}");
        }
        assert_eq!(ColorScheme::default().unique_id(), "urn:microsoft.com/office/officeart/2005/8/colors/accent1_2");
        assert_eq!(QuickStyle::from_unique_id("urn:microsoft.com/office/officeart/2005/8/quickstyle/simple3"), Some(QuickStyle::SubtleEffect));
        let look = |colors, style| {
            let mut art = SmartArt::new(Layout::BlockList);
            (art.colors, art.style) = (colors, style);
            let pieces = art.pieces(&theme(), 576.0, 384.0);
            pieces.iter().map(|piece| (piece.shape.fill.unwrap(), piece.shape.outline.map(|line| (line.color, line.width)), piece.shape.font.color.unwrap())).collect::<Vec<_>>()
        };
        // A new graphic: accent 1 with a white line and white text.
        let plain = look(ColorScheme::default(), QuickStyle::Simple);
        assert!(plain.iter().all(|shape| *shape == (accent(1), Some((LIGHT_1(), 1.0)), LIGHT_1())));
        let outlined = look(ColorScheme::Accent { accent: 2, variant: 1 }, QuickStyle::Simple);
        assert_eq!(outlined[0], (LIGHT_1(), Some((accent(2), 1.0)), DARK_1()));
        // Colourful: one accent after the other, from the second.
        let colourful = look(ColorScheme::Colorful(1), QuickStyle::Simple);
        assert_eq!((colourful[0].0, colourful[1].0, colourful[4].0), (accent(2), accent(3), accent(6)));
        let range = look(ColorScheme::Colorful(3), QuickStyle::Simple);
        assert_eq!((range[0].0, range[4].0), (accent(3), accent(4)));
        let gradient = look(ColorScheme::Accent { accent: 1, variant: 3 }, QuickStyle::Simple);
        assert!(gradient[0].0 != gradient[4].0 && gradient[2].0 != gradient[0].0);
        let looped = look(ColorScheme::Accent { accent: 1, variant: 4 }, QuickStyle::Simple);
        assert_eq!(looped[0].0, looped[4].0);
        // Styles: a heavy white line, a pale fill with dark text, no line.
        assert_eq!(look(ColorScheme::default(), QuickStyle::WhiteOutline)[0].1, Some((LIGHT_1(), 3.0)));
        let subtle = look(ColorScheme::default(), QuickStyle::SubtleEffect)[0];
        assert_eq!((subtle.1, subtle.2), (Some((accent(1), 0.5)), DARK_1()));
        assert_eq!(look(ColorScheme::default(), QuickStyle::ModerateEffect)[0].1, None);
        let mut art = SmartArt::new(Layout::Venn);
        (art.colors, art.style, art.rtl) = (ColorScheme::Colorful(2), QuickStyle::IntenseEffect, true);
        art.reset();
        assert_eq!((art.colors, art.style, art.rtl), (ColorScheme::default(), QuickStyle::Simple, true));
    }

    #[test]
    fn a_graphic_of_a_file_is_shown_through_its_shapes() {
        let mut art = SmartArt::new(Layout::BlockList);
        let piece = art.pieces(&theme(), 400.0, 200.0).remove(0);
        art.foreign = Some(Arc::new(Foreign {
            pieces: vec![piece.clone()],
            size: (400.0, 200.0),
            parts: Default::default(),
        }));
        // Stretched with the frame, whatever the outline says.
        let shown = art.pieces(&theme(), 800.0, 100.0);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].x, shown[0].width, shown[0].height), (piece.x * 2.0, piece.width * 2.0, piece.height / 2.0));
        // Given a layout, it is drawn from its outline again.
        art.set_layout(Layout::Cycle);
        assert_eq!(art.pieces(&theme(), 400.0, 400.0).len(), 10);
    }
}
