//! TypeScript to JavaScript: the types go, enums and parameter properties
//! become code, and a source map says which place of the TypeScript text
//! a place of the JavaScript comes from.

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::Statement;
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::diagnostics::OxcDiagnostic;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};

use crate::{ErrorKind, ScriptError};

/// A script ready to run.
pub(crate) struct Compiled {
    pub js: String,
    /// (line, column) of the JavaScript, then of the TypeScript, 0-based,
    /// in the order of the JavaScript.
    map: Vec<(u32, u32, u32, u32)>,
    /// The parameters `main` declares.
    pub main_params: usize,
    /// Where `main` is (1-based).
    pub main_at: (u32, u32),
}

impl Compiled {
    /// The place of the TypeScript text (1-based line and column) a place
    /// of the JavaScript (1-based too) was written from.
    pub fn origin(&self, line: u32, column: u32) -> Option<(u32, u32)> {
        let key = (line.checked_sub(1)?, column.saturating_sub(1));
        let after = self.map.partition_point(|m| (m.0, m.1) <= key);
        // The piece of code the place is in starts at the last mapping
        // before it; the first of the line when it is before them all.
        let found = match after.checked_sub(1).map(|i| self.map[i]) {
            Some(m) if m.0 == key.0 => m,
            _ => *self.map.get(after).filter(|m| m.0 == key.0)?,
        };
        Some((found.2 + 1, found.3 + 1))
    }
}

/// 1-based line and column of a byte offset.
fn place(source: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(source.len());
    let before = &source.as_bytes()[..offset];
    let line = before.iter().filter(|b| **b == b'\n').count();
    let start = before.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let column = String::from_utf8_lossy(&before[start..]).chars().count();
    (line as u32 + 1, column as u32 + 1)
}

fn syntax_error(source: &str, diagnostic: &OxcDiagnostic) -> ScriptError {
    // When two places are named (a name declared twice), the second is
    // the one at fault.
    let at = diagnostic.labels.iter().last().map(|label| place(source, label.offset() as usize));
    let mut message = diagnostic.message.to_string();
    if let Some(help) = &diagnostic.help {
        message.push_str(". ");
        message.push_str(help);
    }
    ScriptError {
        kind: ErrorKind::Syntax,
        message,
        line: at.map(|p| p.0),
        column: at.map(|p| p.1),
    }
}

/// The name `main` gives its first parameter, when the text can be read
/// that far.
pub(crate) fn workbook_parameter(source: &str) -> Option<String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    parsed.program.body.iter().find_map(|statement| match statement {
        Statement::FunctionDeclaration(f) if f.id.as_ref().is_some_and(|id| id.name == "main") => {
            let name = f.params.items.first()?.pattern.get_identifier_name()?;
            Some(name.to_string())
        }
        _ => None,
    })
}

/// Compile a script, or say what is wrong with its text.
pub(crate) fn compile(source: &str) -> Result<Compiled, Vec<ScriptError>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    if !parsed.diagnostics.is_empty() {
        let mut errors: Vec<ScriptError> = parsed.diagnostics.iter().map(|d| syntax_error(source, d)).collect();
        if errors.is_empty() {
            errors.push(ScriptError {
                kind: ErrorKind::Syntax,
                message: "The script is not valid TypeScript".to_string(),
                line: None,
                column: None,
            });
        }
        return Err(errors);
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(&program);
    if !semantic.diagnostics.is_empty() {
        return Err(semantic.diagnostics.iter().map(|d| syntax_error(source, d)).collect());
    }
    let main = program.body.iter().find_map(|statement| match statement {
        Statement::FunctionDeclaration(f) if f.id.as_ref().is_some_and(|id| id.name == "main") && f.body.is_some() => {
            Some((f.params.items.len() + f.params.rest.is_some() as usize, f.span.start as usize))
        }
        _ => None,
    });
    let Some((main_params, main_offset)) = main else {
        return Err(vec![ScriptError {
            kind: ErrorKind::Syntax,
            message: "The script must have a function named main: function main(workbook: ExcelScript.Workbook) { }".to_string(),
            line: None,
            column: None,
        }]);
    };
    let scoping = semantic.semantic.into_scoping();
    let options = TransformOptions::default();
    let transformed = Transformer::new(&allocator, Path::new("script.ts"), &options).build_with_scoping(scoping, &mut program);
    if !transformed.diagnostics.is_empty() {
        return Err(transformed.diagnostics.iter().map(|d| syntax_error(source, d)).collect());
    }
    let printed = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some("script.ts".into()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let mut map: Vec<(u32, u32, u32, u32)> = printed
        .map
        .map(|map| {
            map.get_tokens()
                .map(|t| (t.get_dst_line(), t.get_dst_col(), t.get_src_line(), t.get_src_col()))
                .collect()
        })
        .unwrap_or_default();
    map.sort_unstable();
    Ok(Compiled {
        js: printed.code,
        map,
        main_params,
        main_at: place(source, main_offset),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_go_and_places_stay() {
        let source = "interface P { x: number }\nenum Color { Red, Green }\nfunction main(workbook: ExcelScript.Workbook) {\n  const p = { x: 1 } as P;\n  boom(p.x as number, Color.Green);\n}\n";
        let compiled = compile(source).expect("compiles");
        assert!(!compiled.js.contains("interface") && !compiled.js.contains(" as "));
        assert_eq!((compiled.main_params, compiled.main_at), (1, (3, 1)));
        let line = compiled.js.lines().position(|l| l.contains("boom(")).expect("the call") as u32;
        let column = compiled.js.lines().nth(line as usize).unwrap().find("boom").unwrap() as u32;
        assert_eq!(compiled.origin(line + 1, column + 1), Some((5, 3)));
    }

    #[test]
    fn what_is_wrong_is_placed() {
        let errors = compile("function main(workbook: ExcelScript.Workbook) {\n  let x = ;\n}\n").err().expect("an error");
        assert_eq!((errors[0].kind, errors[0].line, errors[0].column), (ErrorKind::Syntax, Some(2), Some(11)));
        let errors = compile("function helper() {}\n").err().expect("no main");
        assert!(errors[0].message.contains("main"));
        let errors = compile("function main(workbook) {\n  let a = 1;\n  let a = 2;\n}\n").err().expect("declared twice");
        assert_eq!(errors[0].line, Some(3));
    }
}
