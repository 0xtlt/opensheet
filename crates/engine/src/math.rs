//! Equations: the structures of Office Math as a tree, and the linear
//! format they are typed in.
//!
//! An equation is a row of nodes ([`Math`]): runs of characters and the
//! structures that hold rows of their own (fraction, scripts, radical,
//! n-ary operator, delimiters, function, accent, bar, limit, matrix).
//! That tree is what a file stores (`m:oMath`) and what is drawn.
//!
//! The linear format is the one Office builds equations up from
//! (UnicodeMath): `a/b`, `x^2`, `x_i`, `√(b^2-4ac)`, `∑_(k=0)^n▒a_k`,
//! `sin⁡x`, `■(a&b@c&d)`, with `\alpha`, `\sqrt`, `\sum`… standing for the
//! characters. [`parse`] builds the tree, [`to_linear`] writes it back, and
//! what [`to_linear`] writes [`parse`] reads as the same tree.
//!
//! Left out of the linear format: equation arrays, boxes, phantoms, and
//! the numbering of equations.

/// A node of an equation.
#[derive(Clone, Debug, PartialEq)]
pub enum Math {
    /// Characters: one letter, one number, one operator sign.
    Text(String),
    /// `bar` false: one over the other without a line (a binomial
    /// coefficient).
    Fraction { num: Row, den: Row, bar: bool },
    /// A base with a subscript, a superscript or both.
    Script { base: Row, sub: Option<Row>, sup: Option<Row> },
    Radical { degree: Option<Row>, body: Row },
    /// A large operator with its limits (under and over it, or beside it
    /// as scripts) and what it applies to.
    Nary { op: char, sub: Option<Row>, sup: Option<Row>, limits_beside: bool, body: Row },
    /// Delimiters that grow with what they hold; several items are set
    /// apart by a bar.
    Delimited { open: Option<char>, close: Option<char>, items: Vec<Row> },
    /// A function name set upright, applied to its argument.
    Function { name: Row, arg: Row },
    /// A mark over a body: the combining character says which.
    Accent { mark: char, body: Row },
    /// A line over (or under) a body.
    Bar { over: bool, body: Row },
    /// Text under (or over) a base: `lim` and what it tends to.
    Limit { base: Row, limit: Row, upper: bool },
    Matrix { rows: Vec<Vec<Row>> },
}

pub type Row = Vec<Math>;

/// An equation of a shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Equation {
    pub body: Row,
    /// Shown in the linear format rather than built up ("Linear" of the
    /// Equation tab).
    pub linear: bool,
}

impl Equation {
    /// The equation a text in the linear format builds up to.
    pub fn parse(text: &str) -> Equation {
        Equation {
            body: parse(text),
            linear: false,
        }
    }

    pub fn to_linear(&self) -> String {
        to_linear(&self.body)
    }
}

// ---- characters ----------------------------------------------------------

/// What stands for an empty place of a structure: a dotted square.
pub const PLACEHOLDER: char = '⬚';
/// Between a function name and its argument (invisible).
pub const APPLY: char = '\u{2061}';
/// Between a large operator and what it applies to.
pub const OF: char = '▒';
const GROUP_OPEN: char = '〖';
const GROUP_CLOSE: char = '〗';
const MATRIX: char = '■';
const NO_BAR: char = '¦';
const BELOW: char = '┬';
const ABOVE: char = '┴';
const OVERBAR: char = '¯';
const UNDERBAR: char = '▁';

/// The names typed after a backslash and the characters they stand for.
pub const NAMES: &[(&str, char)] = &[
    ("alpha", 'α'), ("beta", 'β'), ("gamma", 'γ'), ("delta", 'δ'), ("epsilon", 'ϵ'), ("varepsilon", 'ε'),
    ("zeta", 'ζ'), ("eta", 'η'), ("theta", 'θ'), ("vartheta", 'ϑ'), ("iota", 'ι'), ("kappa", 'κ'),
    ("lambda", 'λ'), ("mu", 'μ'), ("nu", 'ν'), ("xi", 'ξ'), ("pi", 'π'), ("rho", 'ρ'), ("sigma", 'σ'),
    ("tau", 'τ'), ("upsilon", 'υ'), ("phi", 'ϕ'), ("varphi", 'φ'), ("chi", 'χ'), ("psi", 'ψ'), ("omega", 'ω'),
    ("Gamma", 'Γ'), ("Delta", 'Δ'), ("Theta", 'Θ'), ("Lambda", 'Λ'), ("Xi", 'Ξ'), ("Pi", 'Π'),
    ("Sigma", 'Σ'), ("Upsilon", 'Υ'), ("Phi", 'Φ'), ("Psi", 'Ψ'), ("Omega", 'Ω'),
    ("infty", '∞'), ("pm", '±'), ("mp", '∓'), ("times", '×'), ("div", '÷'), ("cdot", '⋅'), ("ast", '∗'),
    ("le", '≤'), ("ge", '≥'), ("ne", '≠'), ("approx", '≈'), ("equiv", '≡'), ("sim", '∼'), ("cong", '≅'),
    ("propto", '∝'), ("ll", '≪'), ("gg", '≫'), ("to", '→'), ("rightarrow", '→'), ("leftarrow", '←'),
    ("leftrightarrow", '↔'), ("Rightarrow", '⇒'), ("Leftarrow", '⇐'), ("Leftrightarrow", '⇔'), ("mapsto", '↦'),
    ("partial", '∂'), ("nabla", '∇'), ("forall", '∀'), ("exists", '∃'), ("in", '∈'), ("notin", '∉'),
    ("ni", '∋'), ("subset", '⊂'), ("supset", '⊃'), ("subseteq", '⊆'), ("supseteq", '⊇'), ("cup", '∪'),
    ("cap", '∩'), ("emptyset", '∅'), ("angle", '∠'), ("degree", '°'), ("perp", '⊥'), ("parallel", '∥'),
    ("therefore", '∴'), ("because", '∵'), ("neg", '¬'), ("wedge", '∧'), ("vee", '∨'), ("oplus", '⊕'),
    ("otimes", '⊗'), ("circ", '∘'), ("bullet", '∙'), ("star", '⋆'), ("hbar", 'ℏ'), ("ell", 'ℓ'),
    ("Re", 'ℜ'), ("Im", 'ℑ'), ("aleph", 'ℵ'), ("ldots", '…'), ("cdots", '⋯'), ("vdots", '⋮'), ("ddots", '⋱'),
    ("sqrt", '√'), ("cbrt", '∛'), ("qdrt", '∜'), ("int", '∫'), ("iint", '∬'), ("iiint", '∭'), ("oint", '∮'),
    ("sum", '∑'), ("prod", '∏'), ("coprod", '∐'), ("bigcup", '⋃'), ("bigcap", '⋂'), ("bigvee", '⋁'),
    ("bigwedge", '⋀'), ("hat", '\u{302}'), ("tilde", '\u{303}'), ("bar", '\u{305}'), ("dot", '\u{307}'),
    ("ddot", '\u{308}'), ("vec", '\u{20d7}'), ("check", '\u{30c}'), ("matrix", MATRIX), ("of", OF),
    ("funcapply", APPLY), ("begin", GROUP_OPEN), ("end", GROUP_CLOSE), ("below", BELOW), ("above", ABOVE),
    ("atop", NO_BAR), ("overbar", OVERBAR), ("underbar", UNDERBAR), ("langle", '⟨'), ("rangle", '⟩'),
    ("lfloor", '⌊'), ("rfloor", '⌋'), ("lceil", '⌈'), ("rceil", '⌉'), ("norm", '‖'), ("box", PLACEHOLDER),
];

/// The names set upright, as functions, when they are typed.
pub const FUNCTIONS: &[&str] = &[
    "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "sin", "cos", "tan", "cot", "sec", "csc", "log", "ln",
    "lim", "exp", "min", "max", "det", "gcd", "sup", "inf", "arg", "dim", "ker", "mod",
];

fn is_nary(ch: char) -> bool {
    matches!(ch, '∑' | '∏' | '∐' | '∫' | '∬' | '∭' | '∮' | '⋃' | '⋂' | '⋁' | '⋀')
}

/// Integrals have their limits beside them, the others under and over.
pub fn limits_beside(op: char) -> bool {
    matches!(op, '∫' | '∬' | '∭' | '∮')
}

fn is_accent(ch: char) -> bool {
    matches!(ch, '\u{302}' | '\u{303}' | '\u{305}' | '\u{307}' | '\u{308}' | '\u{20d7}' | '\u{30c}')
}

fn closer(open: char) -> Option<char> {
    Some(match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '⟨' => '⟩',
        '⌊' => '⌋',
        '⌈' => '⌉',
        '|' => '|',
        '‖' => '‖',
        _ => return None,
    })
}

fn is_closer(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '⟩' | '⌋' | '⌉' | GROUP_CLOSE)
}

/// Replace `\name` by its character and the plain-text spellings of a few
/// signs (`->`, `<=`, `>=`, `+-`, `...`) by the signs. A name that is not
/// known stays as typed.
pub fn expand_names(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        if ch == '\\' {
            let name: String = rest[1..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if let Some((_, symbol)) = NAMES.iter().find(|(known, _)| *known == name) {
                out.push(*symbol);
                rest = &rest[1 + name.len()..];
                // The space that ended the name was typed to end it.
                if !is_accent(*symbol) {
                    rest = rest.strip_prefix(' ').unwrap_or(rest);
                }
                continue;
            }
        }
        let sign = [("->", '→'), ("<=", '≤'), (">=", '≥'), ("+-", '±'), ("-+", '∓'), ("...", '…'), ("!=", '≠')]
            .into_iter()
            .find(|(typed, _)| rest.starts_with(typed));
        match sign {
            Some((typed, symbol)) => {
                out.push(symbol);
                rest = &rest[typed.len()..];
            }
            None => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

// ---- reading the linear format -------------------------------------------

struct Parser {
    chars: Vec<char>,
    at: usize,
    /// What a bar does here: 0, it opens `|…|`; 1, it closes it; 2, it
    /// sets the items of other delimiters apart.
    bars: usize,
}

/// The pieces a run of characters is made of: each letter, each number,
/// each other character.
pub fn tokens(text: &str) -> Row {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let start = at;
        if chars[at].is_ascii_digit() {
            while at < chars.len() && (chars[at].is_ascii_digit() || (matches!(chars[at], '.' | ',') && chars.get(at + 1).is_some_and(char::is_ascii_digit))) {
                at += 1;
            }
        } else {
            at += 1;
        }
        out.push(Math::Text(chars[start..at].iter().collect()));
    }
    out
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// True when a bar ends what is being read rather than opening
    /// something: it closes `|…|` or `‖…‖`, or sets two items apart.
    fn bar_ends(&self, ch: char) -> bool {
        (matches!(ch, '|' | '‖') && self.bars == 1) || (ch == '|' && self.bars == 2)
    }

    fn spaces(&mut self) {
        while self.eat(' ') {}
    }

    /// The function name the text goes on with, if it does.
    fn function_name(&self) -> Option<&'static str> {
        let rest: String = self.chars[self.at..].iter().take(8).collect();
        FUNCTIONS.iter().copied().find(|name| {
            // Not the start of a longer word.
            rest.starts_with(name) && !self.chars.get(self.at + name.len()).is_some_and(|c| c.is_ascii_alphabetic())
        })
    }

    /// Everything up to a character that closes what holds it.
    fn expression(&mut self, stop: &[char]) -> Row {
        let mut out = Vec::new();
        while let Some(ch) = self.peek() {
            if stop.contains(&ch) || is_closer(ch) || self.bar_ends(ch) {
                break;
            }
            if ch == ' ' {
                self.at += 1;
                continue;
            }
            let before = self.at;
            out.extend(self.term());
            if self.at == before {
                // A character nothing takes: it stands for itself.
                self.at += 1;
                out.push(Math::Text(ch.to_string()));
            }
        }
        out
    }

    /// Factors side by side, and the fractions made of them: `ab/cd`.
    fn term(&mut self) -> Row {
        let mut left = self.factors();
        loop {
            let bar = match self.peek() {
                Some('/') => true,
                Some(NO_BAR) => false,
                _ => break,
            };
            if left.is_empty() {
                break;
            }
            self.at += 1;
            let right = self.factors();
            left = vec![Math::Fraction {
                num: unwrapped(left),
                den: unwrapped(right),
                bar,
            }];
        }
        left
    }

    /// Operands side by side, up to an operator, a blank or a closer.
    fn factors(&mut self) -> Row {
        let mut out = Vec::new();
        while let Some(node) = self.scripted() {
            // A large operator and a function take what follows them.
            let greedy = matches!(node, Math::Nary { .. } | Math::Function { .. });
            out.push(node);
            if greedy {
                break;
            }
        }
        out
    }

    /// An operand with what is attached to it: scripts, accents, limits,
    /// a factorial sign.
    fn scripted(&mut self) -> Option<Math> {
        let mut node = self.atom()?;
        loop {
            match self.peek() {
                Some(mark) if is_accent(mark) => {
                    self.at += 1;
                    node = Math::Accent { mark, body: body_of(node) };
                }
                Some('_' | '^') => {
                    let (mut sub, mut sup) = (None, None);
                    while let Some(which @ ('_' | '^')) = self.peek() {
                        let slot = if which == '_' { &mut sub } else { &mut sup };
                        if slot.is_some() {
                            break;
                        }
                        self.at += 1;
                        *slot = Some(self.argument());
                    }
                    node = Math::Script { base: body_of(node), sub, sup };
                }
                Some(place @ (BELOW | ABOVE)) => {
                    self.at += 1;
                    node = Math::Limit {
                        base: body_of(node),
                        limit: self.argument(),
                        upper: place == ABOVE,
                    };
                }
                _ => break,
            }
        }
        Some(node)
    }

    /// What a script, a limit or a radical applies to: one operand, or a
    /// group whose brackets do not show.
    fn argument(&mut self) -> Row {
        // A sign before a number or a letter belongs to it: `x^-1`.
        if let Some(sign @ ('-' | '−' | '+')) = self.peek() {
            self.at += 1;
            let mut out = vec![Math::Text(sign.to_string())];
            out.extend(self.argument());
            return out;
        }
        match self.atom() {
            Some(node) => unwrapped(vec![node]),
            None => Vec::new(),
        }
    }

    /// A group in brackets that do not show: `〖…〗`.
    fn hidden_group(&mut self) -> Option<Row> {
        if !self.eat(GROUP_OPEN) {
            return None;
        }
        let saved = std::mem::take(&mut self.bars);
        let inner = self.expression(&[]);
        self.bars = saved;
        self.eat(GROUP_CLOSE);
        Some(inner)
    }

    fn atom(&mut self) -> Option<Math> {
        let ch = self.peek()?;
        if let Some(inner) = self.hidden_group() {
            // Brackets that do not show hold their row together: as a
            // delimited group without delimiters.
            return Some(Math::Delimited {
                open: None,
                close: None,
                items: vec![inner],
            });
        }
        if let Some(close) = closer(ch).filter(|_| !self.bar_ends(ch)) {
            self.at += 1;
            let bars = ch == '|' || ch == '‖';
            let saved = self.bars;
            self.bars = if bars { 1 } else { 2 };
            let mut items = vec![self.expression(if bars { &[] } else { &['|'] })];
            while !bars && self.eat('|') {
                items.push(self.expression(&['|']));
            }
            self.bars = saved;
            // Any closer closes: `[a,b)` is an interval.
            let closed = match self.peek() {
                Some(found) if found == close || (is_closer(found) && found != GROUP_CLOSE) => {
                    self.at += 1;
                    Some(found)
                }
                _ => None,
            };
            return Some(Math::Delimited {
                open: Some(ch),
                close: closed,
                items,
            });
        }
        if matches!(ch, '√' | '∛' | '∜') {
            self.at += 1;
            let degree = match ch {
                '∛' => Some(tokens("3")),
                '∜' => Some(tokens("4")),
                _ => None,
            };
            // `√(n&x)`: the degree before the ampersand.
            if self.peek() == Some('(') {
                self.at += 1;
                let first = self.expression(&['&']);
                let (degree, body) = if self.eat('&') { (Some(first), self.expression(&[])) } else { (degree, first) };
                self.eat(')');
                return Some(Math::Radical { degree, body });
            }
            return Some(Math::Radical { degree, body: self.argument() });
        }
        if is_nary(ch) {
            self.at += 1;
            let (mut sub, mut sup) = (None, None);
            while let Some(which @ ('_' | '^')) = self.peek() {
                self.at += 1;
                *(if which == '_' { &mut sub } else { &mut sup }) = Some(self.argument());
            }
            // After `▒`, one operand; else what follows side by side.
            let body = if self.eat(OF) {
                self.spaces();
                match self.hidden_group() {
                    Some(inner) => inner,
                    None => self.scripted().into_iter().collect(),
                }
            } else {
                self.spaces();
                self.term()
            };
            return Some(Math::Nary {
                op: ch,
                sub,
                sup,
                limits_beside: limits_beside(ch),
                body,
            });
        }
        if ch == MATRIX && self.chars.get(self.at + 1) == Some(&'(') {
            self.at += 2;
            let mut rows = vec![vec![]];
            loop {
                let cell = self.expression(&['&', '@']);
                rows.last_mut().expect("a row").push(cell);
                if self.eat('&') {
                    continue;
                }
                if self.eat('@') {
                    rows.push(Vec::new());
                    continue;
                }
                break;
            }
            self.eat(')');
            return Some(Math::Matrix { rows });
        }
        if matches!(ch, OVERBAR | UNDERBAR) {
            self.at += 1;
            return Some(Math::Bar {
                over: ch == OVERBAR,
                body: self.argument(),
            });
        }
        if let Some(name) = self.function_name() {
            self.at += name.chars().count();
            let mut head = Math::Text(name.to_string());
            // What is attached to the name: `sin^2`, `lim┬(n→∞)`.
            loop {
                match self.peek() {
                    Some('_' | '^') => {
                        let (mut sub, mut sup) = (None, None);
                        while let Some(which @ ('_' | '^')) = self.peek() {
                            self.at += 1;
                            *(if which == '_' { &mut sub } else { &mut sup }) = Some(self.argument());
                        }
                        head = Math::Script { base: vec![head], sub, sup };
                    }
                    Some(place @ (BELOW | ABOVE)) => {
                        self.at += 1;
                        head = Math::Limit {
                            base: vec![head],
                            limit: self.argument(),
                            upper: place == ABOVE,
                        };
                    }
                    _ => break,
                }
            }
            self.eat(APPLY);
            self.spaces();
            let arg = match self.hidden_group() {
                Some(inner) => inner,
                None => self.scripted().into_iter().collect(),
            };
            return Some(Math::Function { name: vec![head], arg });
        }
        if ch.is_ascii_digit() {
            let start = self.at;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || (matches!(c, '.' | ',') && self.chars.get(self.at + 1).is_some_and(char::is_ascii_digit))) {
                self.at += 1;
            }
            return Some(Math::Text(self.chars[start..self.at].iter().collect()));
        }
        // Letters, the placeholder and the signs that follow an operand
        // closely are operands; anything else is for the caller.
        if ch.is_alphabetic() || matches!(ch, PLACEHOLDER | '∞' | '!' | '′' | '°' | '∂' | '∇' | '…' | '⋯' | '∅' | 'ℏ') {
            self.at += 1;
            return Some(Math::Text(ch.to_string()));
        }
        None
    }
}

/// The body of a group when it is the argument of a structure: brackets
/// that do not show are dropped, and so are the parentheses typed around
/// the argument of a fraction, a script or a radical.
fn unwrapped(mut row: Row) -> Row {
    if row.len() == 1 {
        if let Math::Delimited { open, close, items } = &mut row[0] {
            let hidden = open.is_none() && close.is_none();
            let round = *open == Some('(') && *close == Some(')');
            if items.len() == 1 && (hidden || round) {
                return items.remove(0);
            }
        }
    }
    row
}

/// A node as the base of a script or the body of an accent: a hidden
/// group gives its row.
fn body_of(node: Math) -> Row {
    match node {
        Math::Delimited { open: None, close: None, mut items } if items.len() == 1 => items.remove(0),
        node => vec![node],
    }
}

/// Hidden groups that hold nothing together any more are taken apart.
fn flatten(row: Row) -> Row {
    let mut out = Vec::with_capacity(row.len());
    for node in row {
        match node {
            Math::Delimited { open: None, close: None, items } => {
                for item in items {
                    out.extend(flatten(item));
                }
            }
            Math::Text(text) => out.push(Math::Text(text)),
            Math::Fraction { num, den, bar } => out.push(Math::Fraction {
                num: flatten(num),
                den: flatten(den),
                bar,
            }),
            Math::Script { base, sub, sup } => out.push(Math::Script {
                base: flatten(base),
                sub: sub.map(flatten),
                sup: sup.map(flatten),
            }),
            Math::Radical { degree, body } => out.push(Math::Radical {
                degree: degree.map(flatten),
                body: flatten(body),
            }),
            Math::Nary { op, sub, sup, limits_beside, body } => out.push(Math::Nary {
                op,
                sub: sub.map(flatten),
                sup: sup.map(flatten),
                limits_beside,
                body: flatten(body),
            }),
            Math::Delimited { open, close, items } => out.push(Math::Delimited {
                open,
                close,
                items: items.into_iter().map(flatten).collect(),
            }),
            Math::Function { name, arg } => out.push(Math::Function {
                name: flatten(name),
                arg: flatten(arg),
            }),
            Math::Accent { mark, body } => out.push(Math::Accent { mark, body: flatten(body) }),
            Math::Bar { over, body } => out.push(Math::Bar { over, body: flatten(body) }),
            Math::Limit { base, limit, upper } => out.push(Math::Limit {
                base: flatten(base),
                limit: flatten(limit),
                upper,
            }),
            Math::Matrix { rows } => out.push(Math::Matrix {
                rows: rows.into_iter().map(|row| row.into_iter().map(flatten).collect()).collect(),
            }),
        }
    }
    out
}

/// Build an equation up from the linear format.
pub fn parse(text: &str) -> Row {
    let mut parser = Parser {
        chars: expand_names(text).chars().filter(|ch| !matches!(ch, '\n' | '\r' | '\t')).collect(),
        at: 0,
        bars: 0,
    };
    let mut out = Vec::new();
    while parser.at < parser.chars.len() {
        out.extend(parser.expression(&[]));
        // A closer nothing opened stands for itself.
        if let Some(ch) = parser.peek() {
            parser.at += 1;
            out.push(Math::Text(ch.to_string()));
        }
    }
    flatten(out)
}

// ---- writing the linear format -------------------------------------------

/// True when a row is one operand as the reader takes them: it needs no
/// brackets to be the argument of a script or of a fraction.
fn is_operand(row: &Row) -> bool {
    match row.as_slice() {
        [Math::Text(text)] => text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',')) || text.chars().count() == 1 && text.chars().all(|c| c.is_alphabetic() || matches!(c, PLACEHOLDER | '∞')),
        [Math::Delimited { open: Some(_), close: Some(_), .. }] => false,
        _ => false,
    }
}

/// A row as the argument of a script, a radical or a limit: in round
/// brackets (which do not show there) unless it is one operand.
fn argument(row: &Row) -> String {
    if is_operand(row) { to_linear(row) } else { format!("({})", to_linear(row)) }
}

/// A row that must be read as a whole where one operand is expected.
fn grouped(row: &Row) -> String {
    // The name of a function is read as one word.
    let name = matches!(row.as_slice(), [Math::Text(text)] if FUNCTIONS.contains(&text.as_str()));
    if is_operand(row) || name { to_linear(row) } else { format!("{GROUP_OPEN}{}{GROUP_CLOSE}", to_linear(row)) }
}

/// The side of a fraction: what stands side by side needs no brackets.
fn side(row: &Row) -> String {
    let plain = row.iter().all(|node| match node {
        Math::Text(text) => text.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | ',' | '!' | PLACEHOLDER)),
        Math::Script { .. } | Math::Radical { .. } | Math::Accent { .. } => true,
        _ => false,
    });
    if plain && !row.is_empty() { to_linear(row) } else { format!("({})", to_linear(row)) }
}

fn scripts(sub: &Option<Row>, sup: &Option<Row>) -> String {
    let mut out = String::new();
    if let Some(sub) = sub {
        out.push('_');
        out.push_str(&argument(sub));
    }
    if let Some(sup) = sup {
        out.push('^');
        out.push_str(&argument(sup));
    }
    out
}

/// An equation in the linear format.
pub fn to_linear(row: &Row) -> String {
    let mut out = String::new();
    for (i, node) in row.iter().enumerate() {
        // What would run into the node before it is set apart.
        let glued = |text: &str| {
            let first = text.chars().next();
            let last = out.chars().last();
            match (last, first) {
                (Some(a), Some(b)) => (a.is_ascii_digit() && b.is_ascii_digit()) || (a.is_alphabetic() && b.is_alphabetic() && function_at(&out, text)),
                _ => false,
            }
        };
        let piece = match node {
            Math::Text(text) => text.clone(),
            Math::Fraction { num, den, bar } => format!("{}{}{}", side(num), if *bar { '/' } else { NO_BAR }, side(den)),
            Math::Script { base, sub, sup } => format!("{}{}", grouped(base), scripts(sub, sup)),
            Math::Radical { degree: None, body } => format!("√{}", argument(body)),
            Math::Radical { degree: Some(degree), body } => format!("√({}&{})", to_linear(degree), to_linear(body)),
            Math::Nary { op, sub, sup, body, .. } => format!("{op}{}{OF}{GROUP_OPEN}{}{GROUP_CLOSE}", scripts(sub, sup), to_linear(body)),
            Math::Delimited { open, close, items } => {
                let inner: Vec<String> = items.iter().map(to_linear).collect();
                let open = open.map(String::from).unwrap_or_default();
                let close = close.map(String::from).unwrap_or_default();
                format!("{open}{}{close}", inner.join("|"))
            }
            Math::Function { name, arg } => format!("{}{APPLY}{GROUP_OPEN}{}{GROUP_CLOSE}", to_linear(name), to_linear(arg)),
            Math::Accent { mark, body } => format!("{}{mark}", grouped(body)),
            Math::Bar { over, body } => format!("{}{}", if *over { OVERBAR } else { UNDERBAR }, argument(body)),
            Math::Limit { base, limit, upper } => format!("{}{}{}", grouped(base), if *upper { ABOVE } else { BELOW }, argument(limit)),
            Math::Matrix { rows } => {
                let rows: Vec<String> = rows.iter().map(|row| row.iter().map(to_linear).collect::<Vec<_>>().join("&")).collect();
                format!("{MATRIX}({})", rows.join("@"))
            }
        };
        // A fraction takes every operand beside it: one that follows
        // another, or a letter, is set apart by a blank.
        let after_fraction = i > 0 && matches!(row[i - 1], Math::Fraction { .. }) && piece.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '(' || c == '√');
        let before_fraction = matches!(node, Math::Fraction { .. }) && out.chars().last().is_some_and(|c| c.is_alphanumeric() || c == ')' || c == '!');
        if glued(&piece) || after_fraction || before_fraction {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    out
}

/// True when writing `next` right after `written` would spell a function
/// name that neither holds.
fn function_at(written: &str, next: &str) -> bool {
    let tail: String = written.chars().rev().take(7).collect::<Vec<_>>().into_iter().rev().collect();
    let joined = format!("{tail}{next}");
    FUNCTIONS.iter().any(|name| {
        joined.match_indices(name).any(|(at, _)| at < tail.len() && at + name.len() > tail.len())
    })
}

// ---- the galleries -------------------------------------------------------

/// The equations of the Equation menu, in the linear format, by the name
/// Excel gives them.
pub const BUILT_IN: [(&str, &str); 9] = [
    ("Area of Circle", "A=πr^2"),
    ("Binomial Theorem", "(x+a)^n=∑_(k=0)^n▒〖(n¦k) x^k a^(n-k)〗"),
    ("Expansion of a Sum", "(1+x)^n=1+nx/1! +(n(n-1) x^2)/2! +…"),
    ("Fourier Series", "f(x)=a_0+∑_(n=1)^∞▒(a_n cos\u{2061}〖nπx/L〗+b_n sin\u{2061}〖nπx/L〗)"),
    ("Pythagorean Theorem", "a^2+b^2=c^2"),
    ("Quadratic Formula", "x=(-b±√(b^2-4ac))/2a"),
    ("Taylor Expansion", "e^x=1+x/1! +x^2/2! +x^3/3! +…, -∞<x<∞"),
    ("Trig Identity 1", "sin\u{2061}α±sin\u{2061}β=2 sin\u{2061}〖1/2 (α±β)〗 cos\u{2061}〖1/2 (α∓β)〗"),
    ("Trig Identity 2", "cos\u{2061}α+cos\u{2061}β=2 cos\u{2061}〖1/2 (α+β)〗 cos\u{2061}〖1/2 (α-β)〗"),
];

/// The galleries of structures of the Equation tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Structure {
    Fraction,
    Script,
    Radical,
    Integral,
    LargeOperator,
    Bracket,
    Function,
    Accent,
    LimitAndLog,
    Operator,
    Matrix,
}

impl Structure {
    pub const ALL: [Structure; 11] = [
        Structure::Fraction,
        Structure::Script,
        Structure::Radical,
        Structure::Integral,
        Structure::LargeOperator,
        Structure::Bracket,
        Structure::Function,
        Structure::Accent,
        Structure::LimitAndLog,
        Structure::Operator,
        Structure::Matrix,
    ];

    /// What the gallery inserts, in the linear format: structures with
    /// empty places, and a few that are often used filled in.
    pub fn templates(self) -> &'static [&'static str] {
        match self {
            Structure::Fraction => &["⬚/⬚", "⬚¦⬚", "dy/dx", "Δy/Δx", "∂y/∂x", "δy/δx", "π/2"],
            Structure::Script => &["⬚^⬚", "⬚_⬚", "⬚_⬚^⬚", "x_(y^2)", "e^(-iωt)", "x^2", "〖_1^n〗Y"],
            Structure::Radical => &["√⬚", "√(⬚&⬚)", "√(2&⬚)", "√(3&⬚)", "(-b±√(b^2-4ac))/2a", "√(a^2+b^2)"],
            Structure::Integral => &["∫▒⬚", "∫_⬚^⬚▒⬚", "∬▒⬚", "∬_⬚^⬚▒⬚", "∭▒⬚", "∮▒⬚", "∮_⬚^⬚▒⬚", "dx", "dy", "dθ"],
            Structure::LargeOperator => &["∑▒⬚", "∑_⬚^⬚▒⬚", "∑_⬚▒⬚", "∏▒⬚", "∏_⬚^⬚▒⬚", "∐_⬚^⬚▒⬚", "⋃_⬚^⬚▒⬚", "⋂_⬚^⬚▒⬚", "⋁_⬚^⬚▒⬚", "⋀_⬚^⬚▒⬚", "∑_(k=0)^n▒〖(n¦k)〗"],
            Structure::Bracket => &["(⬚)", "[⬚]", "{⬚}", "⟨⬚⟩", "⌊⬚⌋", "⌈⬚⌉", "|⬚|", "‖⬚‖", "(⬚|⬚)", "(n¦k)", "⟨n¦k⟩"],
            Structure::Function => &["sin\u{2061}⬚", "cos\u{2061}⬚", "tan\u{2061}⬚", "csc\u{2061}⬚", "sec\u{2061}⬚", "cot\u{2061}⬚", "sin^(-1)\u{2061}⬚", "cos^(-1)\u{2061}⬚", "tan^(-1)\u{2061}⬚", "sinh\u{2061}⬚", "cosh\u{2061}⬚", "tanh\u{2061}⬚", "sin\u{2061}θ", "cos\u{2061}2x", "tan\u{2061}θ=sin\u{2061}θ/cos\u{2061}θ"],
            Structure::Accent => &["⬚\u{307}", "⬚\u{308}", "⬚\u{302}", "⬚\u{30c}", "⬚\u{303}", "⬚\u{305}", "⬚\u{20d7}", "¯⬚", "▁⬚", "¯A", "¯(ABC)"],
            Structure::LimitAndLog => &["log_⬚\u{2061}⬚", "log\u{2061}⬚", "ln\u{2061}⬚", "lim┬⬚\u{2061}⬚", "min┬⬚\u{2061}⬚", "max┬⬚\u{2061}⬚", "lim┬(n→∞)\u{2061}〖(1+1/n)^n〗", "max┬(0≤x≤1)\u{2061}〖xe^(-x^2)〗"],
            Structure::Operator => &["≔", "==", "+=", "−=", "≝", "≞", "≜", "→┴⬚", "←┴⬚", "→┬⬚", "⇒┴⬚", "→┴yields", "→┴∆"],
            Structure::Matrix => &["■(⬚&⬚)", "■(⬚@⬚)", "■(⬚&⬚&⬚)", "■(⬚@⬚@⬚)", "■(⬚&⬚@⬚&⬚)", "■(⬚&⬚&⬚@⬚&⬚&⬚@⬚&⬚&⬚)", "■(1&0@0&1)", "(■(⬚&⬚@⬚&⬚))", "[■(⬚&⬚@⬚&⬚)]", "(■(1&0&0@0&1&0@0&0&1))", "■(⋯)", "■(⋮)", "■(⋱)"],
        }
    }
}

/// The characters of the Symbols gallery.
pub const SYMBOLS: &str = "±∞=≠∼×÷!∝<≪>≫≤≥∓≅≈≡∀∁∂√∛∜∪∩∅%°℉℃∆∇∃∄∈∋←↑→↓↔∴+−¬αβγδεϵθϑμπρστφωΓΔΘΛΞΠΣΦΨΩ∗∙⋮⋯⋰⋱ℵℶ∎";

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str) -> Math {
        Math::Text(text.to_string())
    }

    fn row(text: &str) -> Row {
        tokens(text)
    }

    #[test]
    fn fractions_scripts_and_radicals() {
        assert_eq!(parse("a/b"), vec![Math::Fraction { num: row("a"), den: row("b"), bar: true }]);
        // Operands side by side go into the fraction; parentheses typed
        // around a side do not show.
        assert_eq!(parse("(a+b)/2c"), vec![Math::Fraction { num: row("a+b"), den: row("2c"), bar: true }]);
        assert_eq!(parse("1+x/2 y"), vec![t("1"), t("+"), Math::Fraction { num: row("x"), den: row("2"), bar: true }, t("y")]);
        // A script takes the operand before it, not the word.
        assert_eq!(parse("xy^2"), vec![t("x"), Math::Script { base: row("y"), sub: None, sup: Some(row("2")) }]);
        assert_eq!(parse("x_i^2"), parse("x^2_i"));
        assert_eq!(parse("x_(i+1)"), vec![Math::Script { base: row("x"), sub: Some(row("i+1")), sup: None }]);
        assert_eq!(parse("e^-x"), vec![Math::Script { base: row("e"), sub: None, sup: Some(row("-x")) }]);
        assert_eq!(parse("\\sqrt x"), vec![Math::Radical { degree: None, body: row("x") }]);
        assert_eq!(parse("√(3&x+1)"), vec![Math::Radical { degree: Some(row("3")), body: row("x+1") }]);
        assert_eq!(parse("∛x"), parse("√(3&x)"));
        // Names typed after a backslash, and the signs of plain text.
        assert_eq!(parse("\\alpha+\\beta -> \\infty"), row("α+β→∞"));
        assert_eq!(parse("a<=b"), row("a≤b"));
        assert_eq!(expand_names("\\foo \\pi r"), "\\foo πr");
        assert_eq!(tokens("12.5x"), vec![t("12.5"), t("x")]);
    }

    #[test]
    fn operators_functions_and_brackets() {
        let sum = parse("\\sum_(k=0)^n\\of a_k");
        assert_eq!(
            sum,
            vec![Math::Nary {
                op: '∑',
                sub: Some(row("k=0")),
                sup: Some(row("n")),
                limits_beside: false,
                body: vec![Math::Script { base: row("a"), sub: Some(row("k")), sup: None }],
            }]
        );
        let Math::Nary { limits_beside, body, .. } = &parse("∫_0^1▒〖x^2 dx〗")[0] else {
            panic!("an integral");
        };
        assert!(*limits_beside && body.len() == 3);
        assert_eq!(parse("sin x"), vec![Math::Function { name: row_text("sin"), arg: row("x") }]);
        assert_eq!(parse("sin\u{2061}〖2x〗+1").len(), 3);
        // A word that only starts like a function is letters.
        assert_eq!(parse("since"), row("since"));
        let limit = parse("lim┬(n→∞)\u{2061}a_n");
        let Math::Function { name, .. } = &limit[0] else {
            panic!("a function");
        };
        assert_eq!(name[0], Math::Limit { base: row_text("lim"), limit: row("n→∞"), upper: false });
        assert_eq!(parse("[a,b)"), vec![Math::Delimited { open: Some('['), close: Some(')'), items: vec![row("a,b")] }]);
        assert_eq!(parse("|x|+1"), vec![Math::Delimited { open: Some('|'), close: Some('|'), items: vec![row("x")] }, t("+"), t("1")]);
        assert_eq!(parse("‖x‖"), vec![Math::Delimited { open: Some('‖'), close: Some('‖'), items: vec![row("x")] }]);
        assert_eq!(parse("(a|b)"), vec![Math::Delimited { open: Some('('), close: Some(')'), items: vec![row("a"), row("b")] }]);
        assert_eq!(parse("(n¦k)"), vec![Math::Delimited { open: Some('('), close: Some(')'), items: vec![vec![Math::Fraction { num: row("n"), den: row("k"), bar: false }]] }]);
        assert_eq!(parse("x\\hat"), vec![Math::Accent { mark: '\u{302}', body: row("x") }]);
        assert_eq!(parse("¯(AB)"), vec![Math::Bar { over: true, body: row("AB") }]);
        assert_eq!(parse("■(a&b@c&d)"), vec![Math::Matrix { rows: vec![vec![row("a"), row("b")], vec![row("c"), row("d")]] }]);
        // What nothing opened or closed stands for itself.
        assert_eq!(parse("a)"), row("a)"));
        assert_eq!(parse("(a"), vec![Math::Delimited { open: Some('('), close: None, items: vec![row("a")] }]);
        assert_eq!(parse(""), Vec::new());
    }

    fn row_text(text: &str) -> Row {
        vec![t(text)]
    }

    #[test]
    fn the_linear_format_reads_what_it_writes() {
        let mut all: Vec<&str> = BUILT_IN.iter().map(|(_, linear)| *linear).collect();
        for structure in Structure::ALL {
            all.extend(structure.templates());
        }
        all.extend(["x_(y^2)^(z_1)", "(a/b)/(c/d)", "a/b c", "2 3", "x sin\u{2061}y", "√(x/2)y", "(1/2)x", "a_1 b", "〖ab〗^2", "f(x)/g(x)", "(a+b)\u{305}"]);
        for linear in all {
            let tree = parse(linear);
            let written = to_linear(&tree);
            assert_eq!(parse(&written), tree, "{linear} → {written}");
            assert_eq!(to_linear(&parse(&written)), written, "{linear}");
        }
        assert_eq!(to_linear(&parse("(a+b)/2c")), "(a+b)/2c");
        assert_eq!(to_linear(&parse("x^(n+1)")), "x^(n+1)");
        assert_eq!(to_linear(&parse("\\sqrt x")), "√x");
        assert_eq!(Equation::parse("a^2+b^2=c^2").to_linear(), "a^2+b^2=c^2");
        // The quadratic formula: one fraction with a radical on top.
        let quadratic = parse(BUILT_IN[5].1);
        assert_eq!(quadratic.len(), 3);
        let Math::Fraction { num, den, .. } = &quadratic[2] else {
            panic!("a fraction");
        };
        assert!(matches!(num.last(), Some(Math::Radical { .. })) && *den == row("2a"));
        assert_eq!(BUILT_IN.len(), 9);
        assert_eq!(SYMBOLS.chars().count(), 81);
    }
}
