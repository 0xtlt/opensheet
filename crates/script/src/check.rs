//! What can be told of a script without running it, from the table of
//! the object model alone: a member a class does not have, a call with
//! the wrong number of arguments, an enum value that does not exist; and,
//! for an editor, the members of what stands before the caret.
//!
//! This is not a TypeScript type checker. Types are followed from
//! `workbook` through calls of the model, bindings that are never
//! assigned again, annotations (`: ExcelScript.Range`), `as` casts, the
//! elements of an array a method of the model returns, and the declared
//! return types of the script's own functions. Everywhere else the type
//! is unknown, and of what is unknown nothing is said: a finding is
//! always something that fails when the script reaches it.

use std::collections::HashMap;
use std::sync::OnceLock;

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, BindingPattern, CallExpression, ChainElement, Expression, ForOfStatement,
    ForStatementLeft, FormalParameter, Function, Program, Statement, StaticMemberExpression,
    TSType, TSTypeName, UnaryExpression, VariableDeclarator,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::Parser;
use oxc::semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc::span::{GetSpan, SourceType};
use oxc::syntax::operator::UnaryOperator;
use oxc::syntax::scope::ScopeFlags;

use crate::api::{API, ENUMS};
use crate::compile::place;
use crate::{ErrorKind, ScriptError};

/// What is known of the value of an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Unknown,
    /// An object of a class of the model, by its place in the table.
    Class(usize),
    Array(Box<Ty>),
    /// The `ExcelScript` namespace itself.
    Namespace,
    /// An enum of the namespace, by its place in the table.
    Enum(usize),
}

/// A member of a class as its signature describes it.
struct Member {
    /// The fewest and the most arguments a call takes (no most for a
    /// rest parameter).
    args: (usize, Option<usize>),
    returns: Ty,
}

fn class_index(name: &str) -> Option<usize> {
    API.iter().position(|class| class.name == name)
}

/// A type written in a signature of the table: `Range`, `Range[]`,
/// `Range | undefined`; anything else is not followed.
fn signature_type(text: &str) -> Ty {
    let text = text.trim();
    let text = text.strip_suffix("| undefined").map_or(text, str::trim);
    if let Some(element) = text.strip_suffix("[]") {
        return Ty::Array(Box::new(signature_type(element)));
    }
    class_index(text).map_or(Ty::Unknown, Ty::Class)
}

/// The end of the parameter list of a signature: the place of the `)`
/// that closes it.
fn params_end(signature: &str) -> Option<(usize, usize)> {
    let open = signature.find('(')?;
    let mut depth = 0;
    for (at, c) in signature.char_indices().skip(open) {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            // The arrow of a function type is not a bracket.
            '>' if signature[..at].ends_with('=') => {}
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, at));
                }
            }
            _ => {}
        }
    }
    None
}

/// The text of the return type of a signature.
fn returns_text(signature: &str) -> &str {
    params_end(signature)
        .and_then(|(_, close)| signature[close + 1..].trim().strip_prefix(':'))
        .map_or("", str::trim)
}

fn parse_member(signature: &str) -> Member {
    let unknown = Member {
        args: (0, None),
        returns: Ty::Unknown,
    };
    let Some((open, close)) = params_end(signature) else {
        return unknown;
    };
    // Parameters, cut at the commas that are not inside a type.
    let mut params: Vec<&str> = Vec::new();
    let (mut depth, mut start) = (0, open + 1);
    let list = &signature[..close];
    for (at, c) in list.char_indices().skip(open + 1) {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            '>' if list[..at].ends_with('=') => {}
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                params.push(&list[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    if !list[start..].trim().is_empty() {
        params.push(&list[start..]);
    }
    let name = |param: &str| {
        param
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let rest = params.iter().any(|param| param.trim().starts_with("..."));
    let required = params
        .iter()
        .filter(|param| !name(param).ends_with('?') && !param.trim().starts_with("..."))
        .count();
    Member {
        args: (required, (!rest).then_some(params.len())),
        returns: signature_type(returns_text(signature)),
    }
}

/// The members of every class, parsed once.
fn members() -> &'static Vec<HashMap<&'static str, Member>> {
    static MEMBERS: OnceLock<Vec<HashMap<&'static str, Member>>> = OnceLock::new();
    MEMBERS.get_or_init(|| {
        API.iter()
            .map(|class| {
                class
                    .members
                    .iter()
                    .map(|m| (m.name, parse_member(m.signature)))
                    .collect()
            })
            .collect()
    })
}

/// The names any object answers without being members of the model (see
/// the prelude): reading them is never a mistake.
const QUIET: [&str; 5] = ["then", "toJSON", "constructor", "toString", "valueOf"];

/// The methods of arrays whose callback takes an element first.
const ELEMENT_CALLBACKS: [&str; 8] = [
    "forEach",
    "map",
    "filter",
    "find",
    "findIndex",
    "some",
    "every",
    "flatMap",
];

struct Checker<'s> {
    source: &'s str,
    scoping: &'s Scoping,
    /// The type of the bindings whose type is certain.
    env: HashMap<SymbolId, Ty>,
    /// The declared return type of the functions of the script.
    returns: HashMap<SymbolId, Ty>,
    findings: Vec<ScriptError>,
    /// Inside the operand of `typeof`: asking whether a member exists is
    /// not using it.
    in_typeof: u32,
    /// Where an expression must end to be the one asked about (for
    /// completion), and the innermost one found there with its type.
    probe: Option<u32>,
    probed: Option<(u32, Ty)>,
}

impl<'s> Checker<'s> {
    fn report(&mut self, offset: u32, message: String) {
        let (line, column) = place(self.source, offset as usize);
        self.findings.push(ScriptError {
            kind: ErrorKind::Check,
            message,
            line: Some(line),
            column: Some(column),
        });
    }

    /// The symbol a pattern binds, when it binds one name.
    fn bound(pattern: &BindingPattern) -> Option<SymbolId> {
        match pattern {
            BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
            _ => None,
        }
    }

    /// A type written in the script: `ExcelScript.Range`, an array of
    /// one, either with `| undefined`.
    fn written(&self, ty: &TSType) -> Ty {
        match ty {
            TSType::TSTypeReference(reference) => match &reference.type_name {
                TSTypeName::QualifiedName(name) => match &name.left {
                    TSTypeName::IdentifierReference(space) if space.name == "ExcelScript" => {
                        class_index(name.right.name.as_str()).map_or(Ty::Unknown, Ty::Class)
                    }
                    _ => Ty::Unknown,
                },
                TSTypeName::IdentifierReference(name) if name.name == "Array" => match reference
                    .type_arguments
                    .as_ref()
                    .map(|args| args.params.as_slice())
                {
                    Some([element]) => Ty::Array(Box::new(self.written(element))),
                    _ => Ty::Unknown,
                },
                _ => Ty::Unknown,
            },
            TSType::TSArrayType(array) => Ty::Array(Box::new(self.written(&array.element_type))),
            TSType::TSParenthesizedType(inner) => self.written(&inner.type_annotation),
            TSType::TSUnionType(union) => {
                let mut held = union.types.iter().filter(|part| {
                    !matches!(
                        part,
                        TSType::TSUndefinedKeyword(_) | TSType::TSNullKeyword(_)
                    )
                });
                match (held.next(), held.next()) {
                    (Some(only), None) => self.written(only),
                    _ => Ty::Unknown,
                }
            }
            _ => Ty::Unknown,
        }
    }

    fn of_member(&self, member: &StaticMemberExpression) -> Ty {
        match self.of(&member.object) {
            Ty::Namespace => ENUMS
                .iter()
                .position(|e| e.name == member.property.name.as_str())
                .map_or(Ty::Unknown, Ty::Enum),
            _ => Ty::Unknown,
        }
    }

    fn of_call(&self, call: &CallExpression) -> Ty {
        match &call.callee {
            Expression::StaticMemberExpression(member) => self.of_method(member),
            Expression::Identifier(name) => {
                let symbol = name
                    .reference_id
                    .get()
                    .and_then(|id| self.scoping.get_reference(id).symbol_id());
                symbol
                    .and_then(|symbol| self.returns.get(&symbol).cloned())
                    .unwrap_or(Ty::Unknown)
            }
            Expression::ParenthesizedExpression(inner) => match &inner.expression {
                Expression::StaticMemberExpression(member) => self.of_method(member),
                _ => Ty::Unknown,
            },
            _ => Ty::Unknown,
        }
    }

    /// What calling a member gives.
    fn of_method(&self, member: &StaticMemberExpression) -> Ty {
        let name = member.property.name.as_str();
        match self.of(&member.object) {
            Ty::Class(class) => members()[class]
                .get(name)
                .map_or(Ty::Unknown, |m| m.returns.clone()),
            // `find` gives an element; what `filter`, `map` or `slice` give
            // is not followed.
            Ty::Array(element) if name == "find" => *element,
            _ => Ty::Unknown,
        }
    }

    /// The type of an expression, as far as it is certain.
    fn of(&self, expression: &Expression) -> Ty {
        match expression {
            Expression::Identifier(name) => match name
                .reference_id
                .get()
                .map(|id| self.scoping.get_reference(id).symbol_id())
            {
                Some(Some(symbol)) => self.env.get(&symbol).cloned().unwrap_or(Ty::Unknown),
                // The global, not a binding of the script.
                Some(None) if name.name == "ExcelScript" => Ty::Namespace,
                _ => Ty::Unknown,
            },
            Expression::StaticMemberExpression(member) => self.of_member(member),
            Expression::ComputedMemberExpression(member) => match self.of(&member.object) {
                Ty::Array(element) => *element,
                _ => Ty::Unknown,
            },
            Expression::CallExpression(call) => self.of_call(call),
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.of_call(call),
                ChainElement::StaticMemberExpression(member) => self.of_member(member),
                ChainElement::ComputedMemberExpression(member) => match self.of(&member.object) {
                    Ty::Array(element) => *element,
                    _ => Ty::Unknown,
                },
                ChainElement::TSNonNullExpression(inner) => self.of(&inner.expression),
                _ => Ty::Unknown,
            },
            Expression::ParenthesizedExpression(inner) => self.of(&inner.expression),
            Expression::TSNonNullExpression(inner) => self.of(&inner.expression),
            Expression::TSSatisfiesExpression(inner) => self.of(&inner.expression),
            Expression::TSAsExpression(cast) => self.written(&cast.type_annotation),
            Expression::AwaitExpression(awaited) => self.of(&awaited.argument),
            Expression::ConditionalExpression(either) => {
                let (a, b) = (self.of(&either.consequent), self.of(&either.alternate));
                if a == b { a } else { Ty::Unknown }
            }
            Expression::LogicalExpression(either) => {
                let (a, b) = (self.of(&either.left), self.of(&either.right));
                if a == b { a } else { Ty::Unknown }
            }
            _ => Ty::Unknown,
        }
    }

    /// What the script declares at its top: the return types of its
    /// functions, wherever they are called from, and the workbook `main`
    /// receives.
    fn declarations(&mut self, program: &Program) {
        for statement in &program.body {
            let Statement::FunctionDeclaration(function) = statement else {
                continue;
            };
            let Some(symbol) = function.id.as_ref().and_then(|id| id.symbol_id.get()) else {
                continue;
            };
            if let Some(declared) = &function.return_type {
                let ty = self.written(&declared.type_annotation);
                self.returns.insert(symbol, ty);
            }
            if function.id.as_ref().is_some_and(|id| id.name == "main") {
                if let Some(first) = function
                    .params
                    .items
                    .first()
                    .filter(|first| first.type_annotation.is_none())
                {
                    if let Some(workbook) = Self::bound(&first.pattern) {
                        self.env.insert(workbook, Ty::Class(0));
                    }
                }
            }
        }
    }
}

impl<'a> Visit<'a> for Checker<'_> {
    fn visit_expression(&mut self, it: &Expression<'a>) {
        if let Some(end) = self.probe {
            let span = it.span();
            if span.end == end
                && self
                    .probed
                    .as_ref()
                    .is_none_or(|(start, _)| span.start >= *start)
            {
                self.probed = Some((span.start, self.of(it)));
            }
        }
        walk::walk_expression(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        walk::walk_variable_declarator(self, it);
        let Some(symbol) = Self::bound(&it.id) else {
            return;
        };
        match (&it.type_annotation, &it.init) {
            (Some(declared), _) => {
                let ty = self.written(&declared.type_annotation);
                self.env.insert(symbol, ty);
            }
            // A binding assigned again later may hold anything by then.
            (None, Some(init)) if !self.scoping.symbol_is_mutated(symbol) => {
                let ty = self.of(init);
                self.env.insert(symbol, ty);
            }
            _ => {}
        }
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        if let (ForStatementLeft::VariableDeclaration(declaration), Ty::Array(element)) =
            (&it.left, self.of(&it.right))
        {
            if let [one] = declaration.declarations.as_slice() {
                if let (Some(symbol), None) = (Self::bound(&one.id), &one.type_annotation) {
                    if !self.scoping.symbol_is_mutated(symbol) {
                        self.env.insert(symbol, *element);
                    }
                }
            }
        }
        walk::walk_for_of_statement(self, it);
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if let (Some(symbol), Some(declared)) = (Self::bound(&it.pattern), &it.type_annotation) {
            let ty = self.written(&declared.type_annotation);
            self.env.insert(symbol, ty);
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        // A function inside another one: its declared return type is
        // known from here on.
        if let (Some(symbol), Some(declared)) = (
            it.id.as_ref().and_then(|id| id.symbol_id.get()),
            &it.return_type,
        ) {
            let ty = self.written(&declared.type_annotation);
            self.returns.insert(symbol, ty);
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        let asking = it.operator == UnaryOperator::Typeof;
        self.in_typeof += asking as u32;
        walk::walk_unary_expression(self, it);
        self.in_typeof -= asking as u32;
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        let name = it.property.name.as_str();
        match self.of(&it.object) {
            Ty::Class(class)
                if !members()[class].contains_key(name)
                    && !QUIET.contains(&name)
                    && self.in_typeof == 0 =>
            {
                self.report(it.property.span.start, format!("{}.{name} is not supported in OpenSheet: no such member, or one of Office Scripts that OpenSheet does not have", API[class].name));
            }
            Ty::Namespace if !ENUMS.iter().any(|e| e.name == name) && !QUIET.contains(&name) => {
                self.report(it.property.span.start, format!("ExcelScript.{name} is not supported in OpenSheet: no such enum, or one OpenSheet does not have"));
            }
            Ty::Enum(of)
                if !ENUMS[of].members.iter().any(|(member, _)| *member == name)
                    && !QUIET.contains(&name) =>
            {
                self.report(it.property.span.start, format!("ExcelScript.{}.{name} is not supported in OpenSheet: no such value, or one OpenSheet does not have", ENUMS[of].name));
            }
            _ => {}
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(member) = &it.callee {
            let name = member.property.name.as_str();
            match self.of(&member.object) {
                Ty::Class(class) => {
                    let spread = it
                        .arguments
                        .iter()
                        .any(|argument| matches!(argument, Argument::SpreadElement(_)));
                    if let (Some(known), false) = (members()[class].get(name), spread) {
                        let (given, (fewest, most)) = (it.arguments.len(), known.args);
                        if given < fewest || most.is_some_and(|most| given > most) {
                            let expected = match most {
                                Some(most) if most == fewest => format!("{fewest}"),
                                Some(most) => format!("{fewest} to {most}"),
                                None => format!("at least {fewest}"),
                            };
                            let plural = if expected == "1" { "" } else { "s" };
                            self.report(
                                member.property.span.start,
                                format!(
                                    "{}.{name} takes {expected} argument{plural}, not {given}",
                                    API[class].name
                                ),
                            );
                        }
                    }
                }
                // The callback of a method of an array of the model takes
                // an element of it.
                Ty::Array(element) if ELEMENT_CALLBACKS.contains(&name) => {
                    let first = match it.arguments.first() {
                        Some(Argument::ArrowFunctionExpression(arrow)) => {
                            arrow.params.items.first()
                        }
                        Some(Argument::FunctionExpression(function)) => {
                            function.params.items.first()
                        }
                        _ => None,
                    };
                    if let Some(parameter) =
                        first.filter(|parameter| parameter.type_annotation.is_none())
                    {
                        if let Some(symbol) = Self::bound(&parameter.pattern)
                            .filter(|symbol| !self.scoping.symbol_is_mutated(*symbol))
                        {
                            self.env.insert(symbol, *element);
                        }
                    }
                }
                _ => {}
            }
        }
        walk::walk_call_expression(self, it);
    }
}

/// Walk a script; `probe` asks for the type of the expression that ends
/// at an offset.
fn examine(source: &str, probe: Option<u32>) -> Option<(Vec<ScriptError>, Option<Ty>)> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    // A text the parser gave up on has no statement left to look at.
    if parsed.fatal_error {
        return None;
    }
    let semantic = SemanticBuilder::new().build(&parsed.program);
    let scoping = semantic.semantic.scoping();
    let mut checker = Checker {
        source,
        scoping,
        env: HashMap::new(),
        returns: HashMap::new(),
        findings: Vec::new(),
        in_typeof: 0,
        probe,
        probed: None,
    };
    checker.declarations(&parsed.program);
    checker.visit_program(&parsed.program);
    let mut findings = checker.findings;
    findings.sort_by_key(|finding| (finding.line, finding.column));
    Some((findings, checker.probed.map(|(_, ty)| ty)))
}

/// What is wrong in a script that parses: see the top of this file.
pub(crate) fn check(source: &str) -> Vec<ScriptError> {
    examine(source, None)
        .map(|(findings, _)| findings)
        .unwrap_or_default()
}

// ---- completion --------------------------------------------------------------

/// What a name offered by [`complete`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// A method of a class of the model.
    Method,
    /// An enum of the `ExcelScript` namespace.
    Enum,
    /// A member of an enum.
    EnumMember,
}

/// One name an editor can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub name: &'static str,
    pub kind: CompletionKind,
    /// A method as it is declared (`getCell(row: number, column: number):
    /// Range`), an enum member with its value (`center = "Center"`), the
    /// name of an enum.
    pub signature: String,
    /// What a method returns (`Range`, `string[][]`, `void`); empty for
    /// the rest.
    pub returns: String,
    /// A line about a method; empty for the rest.
    pub description: &'static str,
}

/// What can follow the dot before a place of a script.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completions {
    /// Where the name being typed starts (a byte offset): what stands
    /// between it and the place asked about is its beginning, and an
    /// editor replaces from here.
    pub start: usize,
    /// The class of the model whose members are offered (`Range`), or
    /// `ExcelScript` and `ExcelScript.ChartType` for the namespace and
    /// its enums; `None` when what stands before the dot is not known.
    pub of: Option<String>,
    /// Every name that can follow, in the order of the table; an editor
    /// keeps those that start as the name being typed does. Empty when
    /// the type is not known.
    pub items: Vec<Completion>,
}

/// The members of a class of the model, for an editor to offer when it
/// knows the class by other means than [`complete`]; empty for a name
/// that is not a class.
pub fn members_of(class: &str) -> Vec<Completion> {
    let Some(index) = class_index(class) else {
        return Vec::new();
    };
    let method = |member: &'static crate::ApiMember| Completion {
        name: member.name,
        kind: CompletionKind::Method,
        signature: member.signature.to_string(),
        returns: returns_text(member.signature).to_string(),
        description: member.description,
    };
    API[index].members.iter().map(method).collect()
}

fn plain(name: &'static str, kind: CompletionKind, signature: String) -> Completion {
    Completion {
        name,
        kind,
        signature,
        returns: String::new(),
        description: "",
    }
}

/// What can follow the dot that stands before `offset` (a byte offset in
/// `source`, at the caret): the members of the class of the expression
/// before the dot when it is known, the enums after `ExcelScript.`, the
/// members of an enum after `ExcelScript.ChartType.`. The name already
/// typed after the dot is not looked at: `Completions::start` says where
/// it begins. The text may be unfinished, as it is while it is typed.
pub fn complete(source: &str, offset: usize) -> Completions {
    let mut at = offset.min(source.len());
    while !source.is_char_boundary(at) {
        at -= 1;
    }
    let before = &source[..at];
    let start = before
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .map_or(0, |last| {
            last + before[last..].chars().next().map_or(1, char::len_utf8)
        });
    let mut out = Completions {
        start,
        ..Completions::default()
    };
    // The dot, which `?.` and white space may dress.
    let head = before[..start].trim_end();
    let Some(receiver) = head.strip_suffix('.') else {
        return out;
    };
    let receiver = receiver.strip_suffix('?').unwrap_or(receiver).trim_end();
    // The script without the dot and what follows it up to the caret: it
    // then reads as it did before the dot was typed.
    let patched = format!("{receiver}{}", &source[at..]);
    let Some((_, Some(ty))) = examine(&patched, Some(receiver.len() as u32)) else {
        return out;
    };
    match ty {
        Ty::Class(class) => {
            out.of = Some(API[class].name.to_string());
            out.items = members_of(API[class].name);
        }
        Ty::Namespace => {
            out.of = Some("ExcelScript".to_string());
            out.items = ENUMS
                .iter()
                .map(|e| plain(e.name, CompletionKind::Enum, e.name.to_string()))
                .collect();
        }
        Ty::Enum(of) => {
            out.of = Some(format!("ExcelScript.{}", ENUMS[of].name));
            out.items = ENUMS[of]
                .members
                .iter()
                .map(|(name, value)| {
                    plain(
                        name,
                        CompletionKind::EnumMember,
                        format!("{name} = {value:?}"),
                    )
                })
                .collect();
        }
        Ty::Array(_) | Ty::Unknown => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_are_read() {
        let member = |signature: &str| parse_member(signature);
        assert_eq!(
            member("getCell(row: number, column: number): Range").args,
            (2, Some(2))
        );
        assert_eq!(
            member("getRange(address?: string): Range").args,
            (0, Some(1))
        );
        assert_eq!(member("sortByValues(sortBy: ExcelScript.SortBy, valuesHierarchy: DataPivotHierarchy, pivotItemScope?: Array<PivotItem | string>): void").args, (2, Some(3)));
        assert_eq!(
            member("apply(fields: ExcelScript.SortField[], matchCase?: boolean): void").args,
            (1, Some(2))
        );
        // Arrays of plain values are arrays of what is not followed.
        assert_eq!(
            member("getValues(): (string | number | boolean)[][]").returns,
            Ty::Array(Box::new(Ty::Array(Box::new(Ty::Unknown))))
        );
        assert_eq!(member("getName(): string").returns, Ty::Unknown);
        assert_eq!(
            member("getTables(): Table[]").returns,
            Ty::Array(Box::new(Ty::Class(class_index("Table").unwrap())))
        );
        assert_eq!(
            member("getTable(key: string): Table | undefined").returns,
            Ty::Class(class_index("Table").unwrap())
        );
        assert_eq!(
            returns_text("getValues(): (string | number | boolean)[][]"),
            "(string | number | boolean)[][]"
        );
        // Every signature of the table is one that reads.
        for class in API {
            for m in class.members {
                assert!(
                    params_end(m.signature).is_some() && !returns_text(m.signature).is_empty(),
                    "{}.{}",
                    class.name,
                    m.signature
                );
            }
        }
    }
}
