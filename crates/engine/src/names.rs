//! Defined names: scopes, validation and what the Name Manager does with
//! them (new, edit, rename, delete).
//!
//! A name belongs either to the workbook (`Workbook::names`) or to one
//! sheet (`Sheet::names`). From a cell, the name of its own sheet wins
//! over the workbook's; `Sheet2!Name` reaches the name of another sheet.

use crate::ast::{Expr, same_name};
use crate::features::Rewritten;
use crate::locale::Locale;
use crate::parser::{ParseError, parse_formula};
use crate::refs::{Reference, parse_cell_name};
use crate::workbook::{Content, DefinedName, Workbook};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Where a defined name is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameScope {
    Workbook,
    /// One sheet, by its current index.
    Sheet(usize),
}

/// Why a name could not be created or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    /// Not a name Excel accepts: it must start with a letter, `_` or `\`,
    /// continue with letters, digits, `_`, `.`, `?` or `\`, and be neither
    /// a cell reference nor the name of a table.
    Invalid,
    /// The scope already has a name spelled this way (case is ignored).
    Duplicate,
    /// No such name, or no such sheet.
    NotFound,
}

/// Where a formula is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaSite {
    Cell {
        sheet: usize,
        row: u32,
        col: u32,
    },
    /// The definition of a name of that scope.
    Name(NameScope),
    /// A formula a feature of that sheet keeps outside the cells: that of
    /// a conditional format or of a validation rule, or the totals
    /// formula a table remembers while its totals row is hidden.
    Sheet(usize),
}

impl FormulaSite {
    /// (sheet, row, col) of a cell formula.
    pub fn cell(self) -> Option<(usize, u32, u32)> {
        match self {
            FormulaSite::Cell { sheet, row, col } => Some((sheet, row, col)),
            FormulaSite::Name(_) | FormulaSite::Sheet(_) => None,
        }
    }

    /// The sheet from which the formula looks names up, if it has one.
    pub fn sheet(self) -> Option<usize> {
        match self {
            FormulaSite::Cell { sheet, .. }
            | FormulaSite::Name(NameScope::Sheet(sheet))
            | FormulaSite::Sheet(sheet) => Some(sheet),
            FormulaSite::Name(NameScope::Workbook) => None,
        }
    }
}

/// "R", "C", "RC", "R12", "C3", "R1C1": references in the R1C1 notation,
/// which names may not look like either. `row` and `col` are the letters
/// of the notation (uppercase).
fn is_r1c1_reference(name: &str, row: char, col: char) -> bool {
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let upper = name.to_uppercase();
    let after_row = upper
        .strip_prefix(row)
        .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()));
    match (after_row, upper.strip_prefix(col)) {
        (Some(""), _) => true,
        (Some(rest), _) => rest.strip_prefix(col).is_some_and(digits),
        (None, Some(rest)) => digits(rest),
        (None, None) => false,
    }
}

/// Whether a defined name is one Excel accepts: it starts with a letter,
/// `_` or `\`, continues with letters, digits, `_`, `.`, `?` or `\`, is at
/// most 255 characters long and cannot be mistaken for a cell reference
/// (`A1`, `R1C1`) or a boolean. The R1C1 letters and booleans of `locale`
/// are refused as well as the English ones, so that a name stays valid
/// whatever the language of the reader.
pub fn is_valid_name(name: &str, locale: &Locale) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '\\');
    starts_well
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '?' | '\\'))
        && name.chars().count() <= 255
        && parse_cell_name(name).is_none()
        && !is_r1c1_reference(name, 'R', 'C')
        && !is_r1c1_reference(name, locale.r1c1.0, locale.r1c1.1)
        && locale.parse_bool(name).is_none()
}

impl DefinedName {
    /// What the name stands for when used in the cell (`row`, `col`):
    /// relative references are offsets from that cell.
    pub fn expr_at(&self, row: u32, col: u32) -> Expr {
        self.expr.rebased(row as i64, col as i64)
    }

    /// The "refers to" text of the Name Manager (with its `=`), as it reads
    /// when the cell (`row`, `col`) is selected.
    pub fn refers_to(&self, locale: &Locale, row: u32, col: u32) -> String {
        format!("={}", self.expr_at(row, col).to_text(locale))
    }
}

impl Workbook {
    fn scope_names(&self, scope: NameScope) -> Option<&BTreeMap<String, DefinedName>> {
        match scope {
            NameScope::Workbook => Some(&self.names),
            NameScope::Sheet(i) => self.sheets.get(i).map(|s| &s.names),
        }
    }

    fn scope_names_mut(&mut self, scope: NameScope) -> Option<&mut BTreeMap<String, DefinedName>> {
        match scope {
            NameScope::Workbook => Some(&mut self.names),
            NameScope::Sheet(i) => self.sheets.get_mut(i).map(|s| &mut s.names),
        }
    }

    /// The name spelled `name` (case is ignored) in exactly that scope.
    pub fn name_in(&self, scope: NameScope, name: &str) -> Option<&DefinedName> {
        self.scope_names(scope)?.get(&name.to_uppercase())
    }

    /// The name a formula on `sheet` means by `name`: the sheet's own
    /// first, else the workbook's.
    pub fn lookup_name(&self, sheet: usize, name: &str) -> Option<(NameScope, &DefinedName)> {
        let key = name.to_uppercase();
        if let Some(def) = self.sheets.get(sheet).and_then(|s| s.names.get(&key)) {
            return Some((NameScope::Sheet(sheet), def));
        }
        self.names.get(&key).map(|def| (NameScope::Workbook, def))
    }

    /// Every defined name with its scope, as the Name Manager lists them:
    /// by name, the workbook's before the sheets'.
    pub fn all_names(&self) -> Vec<(NameScope, &DefinedName)> {
        let mut out: Vec<(NameScope, &DefinedName)> = self
            .names
            .values()
            .map(|d| (NameScope::Workbook, d))
            .collect();
        for (i, sh) in self.sheets.iter().enumerate() {
            out.extend(sh.names.values().map(|d| (NameScope::Sheet(i), d)));
        }
        out.sort_by_cached_key(|(scope, def)| (def.name.to_uppercase(), *scope));
        out
    }

    /// The cells a name stands for when used in the cell (`sheet`, `row`,
    /// `col`), if it stands for one range: what the Name Box selects. The
    /// name of a table gives its data rows.
    pub fn name_reference(
        &self,
        sheet: usize,
        row: u32,
        col: u32,
        name: &str,
    ) -> Option<Reference> {
        let Some((_, def)) = self.lookup_name(sheet, name) else {
            let (table_sheet, table) = self.table_by_name(name)?;
            return Some(Reference::new(table_sheet, table.data_range()?));
        };
        let mut expr = def.expr_at(row, col);
        while let Expr::Paren(inner) = expr {
            expr = *inner;
        }
        match expr {
            Expr::Ref(r) => {
                let on = match &r.sheet {
                    Some(s) => self.sheet_index(s)?,
                    None => sheet,
                };
                Some(Reference::new(on, r.rect()))
            }
            _ => None,
        }
    }

    /// Check that `name` can be given to a name of `scope`. `replacing` is
    /// the name being renamed or redefined, which is allowed to keep its
    /// spelling.
    pub fn check_name(
        &self,
        scope: NameScope,
        name: &str,
        replacing: Option<&str>,
    ) -> Result<(), NameError> {
        let names = self.scope_names(scope).ok_or(NameError::NotFound)?;
        // Tables share the namespace of names.
        if !is_valid_name(name, self.locale) || self.table_by_name(name).is_some() {
            return Err(NameError::Invalid);
        }
        let taken = names.contains_key(&name.to_uppercase());
        if taken && !replacing.is_some_and(|r| same_name(r, name)) {
            return Err(NameError::Duplicate);
        }
        Ok(())
    }

    /// Parse the "refers to" text of a name entered while the cell
    /// (`sheet`, `row`, `col`) is selected into the form names are stored
    /// in: references without a sheet get that sheet (a name always says
    /// where its cells are), and relative references are kept as offsets
    /// from the cell using the name.
    pub fn parse_name_formula(
        &self,
        text: &str,
        sheet: usize,
        row: u32,
        col: u32,
    ) -> Result<Expr, ParseError> {
        let src = text.trim();
        let src = src.strip_prefix('=').unwrap_or(src);
        let expr = parse_formula(src, self.locale)?;
        let expr = match self.sheets.get(sheet) {
            Some(sh) => expr.with_default_sheet(&sh.name),
            None => expr,
        };
        Ok(expr.rebased(-(row as i64), -(col as i64)))
    }

    /// Create a name ("New Name"). `expr` is in stored form (see
    /// `parse_name_formula`). Takes the undo snapshot; the caller commits.
    pub fn add_name(
        &mut self,
        scope: NameScope,
        name: &str,
        expr: Expr,
        comment: &str,
    ) -> Result<(), NameError> {
        let name = name.trim();
        self.check_name(scope, name, None)?;
        self.snapshot();
        let def = DefinedName {
            name: name.to_string(),
            expr: Arc::new(expr),
            comment: comment.to_string(),
        };
        if let Some(names) = self.scope_names_mut(scope) {
            names.insert(name.to_uppercase(), def);
        }
        Ok(())
    }

    /// Change a name ("Edit Name"): its spelling, what it stands for and
    /// its comment. When the spelling changes, the formulas using the name
    /// follow. The scope of a name cannot be changed, as in Excel.
    pub fn edit_name(
        &mut self,
        scope: NameScope,
        name: &str,
        new_name: &str,
        expr: Expr,
        comment: &str,
    ) -> Result<(), NameError> {
        self.rename_name(scope, name, new_name)?;
        self.snapshot();
        let def = self
            .scope_names_mut(scope)
            .and_then(|names| names.get_mut(&new_name.trim().to_uppercase()))
            .ok_or(NameError::NotFound)?;
        def.expr = Arc::new(expr);
        def.comment = comment.to_string();
        Ok(())
    }

    /// Rename a name and rewrite the formulas (of cells and of other
    /// names) that use it.
    pub fn rename_name(&mut self, scope: NameScope, old: &str, new: &str) -> Result<(), NameError> {
        let new = new.trim();
        let current = self
            .name_in(scope, old)
            .ok_or(NameError::NotFound)?
            .name
            .clone();
        if current == new {
            return Ok(());
        }
        self.check_name(scope, new, Some(&current))?;
        self.snapshot();

        // Which uses of the old spelling mean this very name: from a sheet
        // with a name of its own spelled the same, the bare name is that
        // other one.
        let scope_sheet = match scope {
            NameScope::Sheet(i) => Some(i),
            NameScope::Workbook => None,
        };
        let key = current.to_uppercase();
        let new_arc: Arc<str> = Arc::from(new);
        self.rewrite_formulas(&|wb, site, e| {
            let shadowed = |sheet: usize| {
                wb.sheets
                    .get(sheet)
                    .is_some_and(|s| s.names.contains_key(&key))
            };
            let means_this = |from: Option<usize>| match (scope_sheet, from) {
                (Some(own), from) => from == Some(own),
                (None, Some(from)) => !shadowed(from),
                (None, None) => true,
            };
            // `LET(x, ...)` makes `x` a variable of that formula.
            if e.bound_names().iter().any(|b| same_name(b, &current)) {
                return None;
            }
            Some(e.map(&mut |x| match x {
                Expr::Name(n) if same_name(&n, &current) && means_this(site.sheet()) => {
                    Expr::Name(new_arc.clone())
                }
                Expr::SheetName(sheet, n)
                    if same_name(&n, &current) && means_this(wb.sheet_index(&sheet)) =>
                {
                    Expr::SheetName(sheet, new_arc.clone())
                }
                other => other,
            }))
        });

        if let Some(names) = self.scope_names_mut(scope)
            && let Some(mut def) = names.remove(&key)
        {
            def.name = new.to_string();
            names.insert(new.to_uppercase(), def);
        }
        Ok(())
    }

    /// Delete a name. The formulas using it are left as they are and show
    /// `#NAME?`, as in Excel.
    pub fn delete_name(&mut self, scope: NameScope, name: &str) -> bool {
        if self.name_in(scope, name).is_none() {
            return false;
        }
        self.snapshot();
        self.scope_names_mut(scope)
            .is_some_and(|names| names.remove(&name.to_uppercase()).is_some())
    }

    /// Rewrite the formulas of the workbook: those of the cells, those of
    /// the defined names and those the features keep (conditional formats,
    /// validation rules, totals of tables). `f` is given the workbook as
    /// it is before any change, where the formula is stored and the
    /// formula; it returns the new formula, or `None` to leave it alone.
    ///
    /// The undo snapshot is taken when something changes; the caller
    /// commits. Returns whether something changed.
    pub fn rewrite_formulas(
        &mut self,
        f: &dyn Fn(&Workbook, FormulaSite, &Expr) -> Option<Expr>,
    ) -> bool {
        enum Slot {
            Cell(usize, u32, u32),
            Name(NameScope, String),
        }
        let mut changes: Vec<(Slot, Expr)> = Vec::new();
        let mut kept: Vec<(usize, Rewritten)> = Vec::new();
        {
            let wb: &Workbook = self;
            let mut visit = |slot: Slot, site: FormulaSite, e: &Expr| {
                if let Some(new) = f(wb, site, e)
                    && new != *e
                {
                    changes.push((slot, new));
                }
            };
            for (si, sh) in wb.sheets.iter().enumerate() {
                for ((row, col), cell) in sh.cells.iter() {
                    if let Content::Formula(e) = &cell.content {
                        let site = FormulaSite::Cell {
                            sheet: si,
                            row: *row,
                            col: *col,
                        };
                        visit(Slot::Cell(si, *row, *col), site, e);
                    }
                }
                for (key, def) in sh.names.iter() {
                    let scope = NameScope::Sheet(si);
                    visit(
                        Slot::Name(scope, key.clone()),
                        FormulaSite::Name(scope),
                        &def.expr,
                    );
                }
                let rewritten = Rewritten::of(sh, &mut |e| f(wb, FormulaSite::Sheet(si), e));
                kept.extend(rewritten.map(|r| (si, r)));
            }
            for (key, def) in wb.names.iter() {
                let scope = NameScope::Workbook;
                visit(
                    Slot::Name(scope, key.clone()),
                    FormulaSite::Name(scope),
                    &def.expr,
                );
            }
        }
        if changes.is_empty() && kept.is_empty() {
            return false;
        }
        self.snapshot();
        if !kept.is_empty() {
            for (si, rewritten) in kept {
                rewritten.store(&mut self.sheets[si]);
            }
            self.caches.cond.invalidate();
        }
        for (slot, new) in changes {
            match slot {
                Slot::Cell(si, row, col) => {
                    if let Some(cell) = self.sheets[si].cells.get_mut(&(row, col)) {
                        cell.content = Content::Formula(Arc::new(new));
                    }
                }
                Slot::Name(scope, key) => {
                    if let Some(def) = self
                        .scope_names_mut(scope)
                        .and_then(|names| names.get_mut(&key))
                    {
                        def.expr = Arc::new(new);
                    }
                }
            }
        }
        true
    }
}
