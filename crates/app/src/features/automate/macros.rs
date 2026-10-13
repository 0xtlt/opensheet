//! VBA macros: kept and shown, not run.
//!
//! A macro-enabled workbook keeps its VBA project (see the engine's
//! `vba.rs`). Here is what the window does with it: the message bar that
//! says so when such a workbook is opened, the Macro dialog of View ▸
//! Macros, the read-only viewer its Edit button opens, and Excel's
//! question when the workbook is about to be saved without its macros.
//!
//! What cannot work here is disabled rather than faked: Delete, Options
//! and Create would change the project, which OpenSheet never does. Run
//! and Step Into say once, plainly, that macros are not run, and point to
//! the Automate tab.

use super::editor::CodeEditor;
use super::syntax::Language;
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::view::icon;
use crate::widgets::{self, Button, ListBox, Select};
use gpui::{AnyElement, Context, Entity, FontWeight, SharedString, Subscription, Window, div, prelude::*, px};
use opensheet_io::vba::{Macro, ModuleKind, Project, read_project};
use std::path::Path;
use std::rc::Rc;

/// What the window keeps about the macros of its workbook.
#[derive(Default)]
pub(crate) struct MacroUi {
    /// The project, read once: its modules, or why it cannot be read.
    project: Option<Result<Rc<Project>, String>>,
    /// The message bar was closed.
    pub bar_closed: bool,
}

/// The modules of the VBA project of the workbook; `None` when it has no
/// project, an error when the project cannot be read.
pub(crate) fn project(app: &mut ExcelApp) -> Option<Result<Rc<Project>, String>> {
    let bin = app.wb.vba.project.as_ref()?.bin.clone();
    let ui = app.ext_mut::<MacroUi>();
    Some(ui.project.get_or_insert_with(|| read_project(&bin).map(Rc::new)).clone())
}

/// The one message about running macros.
pub(crate) fn not_run_alert() -> Alert {
    Alert::new(
        "OpenSheet",
        t(
            "OpenSheet keeps the VBA macros of this workbook, but it can't run them. To automate a workbook in OpenSheet, use Office Scripts on the Automate tab.",
            "OpenSheet conserve les macros VBA de ce classeur, mais ne peut pas les exécuter. Pour automatiser un classeur dans OpenSheet, utilisez les scripts Office de l'onglet Automatiser.",
        ),
    )
    .button(t("Show the Automate Tab", "Afficher l'onglet Automatiser"), |app, _, cx| super::show_tab(app, cx))
    .close_button(t("OK", "OK"))
    .default_button(1)
}

// --------------------------------------------------------------- message bar

/// The bar under the ribbon of a workbook that came with macros, until it
/// is closed.
pub(crate) fn message_bar(app: &mut ExcelApp, cx: &mut Cx) -> Option<AnyElement> {
    if !app.wb.has_vba_project() || app.ext::<MacroUi>().is_some_and(|ui| ui.bar_closed) {
        return None;
    }
    Some(
        div()
            .id("macro-bar")
            .debug_selector(|| "macro-bar".to_string())
            .w_full()
            .flex_none()
            .px(px(12.))
            .py(px(5.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(c(0xFFF4CE))
            .border_b_1()
            .border_color(c(0xE5D29A))
            .child(icon("automate-macro", 16.0, 0x8A6100))
            .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).child(t("Macros have been disabled.", "Les macros ont été désactivées.")))
            .child(
                div().flex_1().min_w(px(0.)).truncate().child(t(
                    "OpenSheet keeps the macros of this workbook in the file and does not run them.",
                    "OpenSheet conserve les macros de ce classeur dans le fichier et ne les exécute pas.",
                )),
            )
            .child(
                div().debug_selector(|| "macro-bar-view".to_string()).child(
                    Button::new("macro-bar-view", t("View Macros", "Afficher les macros"))
                        .on_click(cx.listener(|app, _, window, cx| {
                            open_dialog(app, window, cx);
                        })),
                ),
            )
            .child(
                div()
                    .id("macro-bar-close")
                    .debug_selector(|| "macro-bar-close".to_string())
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(0xF0DFA8)))
                    .child(icon("close", 11.0, theme::TEXT_MUTED))
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.ext_mut::<MacroUi>().bar_closed = true;
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

// -------------------------------------------------------------- Macro dialog

/// What "Macros in" offers: every open workbook, or this one. There is
/// one workbook to a window, so both list the same macros.
const SCOPES: usize = 2;

pub(crate) struct MacroDialog {
    modal: Modal,
    /// The title of the workbook, for "Macros in".
    workbook: String,
    project: Option<Rc<Project>>,
    /// Why the project could not be read.
    pub(crate) trouble: Option<String>,
    pub(crate) macros: Vec<Macro>,
    pub(crate) name: Entity<TextInput>,
    pub(crate) scope: usize,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn open_dialog(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<MacroDialog> {
    app.popup = None;
    let project = project(app);
    let workbook = app.title.clone();
    app.open_modal(window, cx, |modal, _, cx| MacroDialog::new(modal, workbook, project, cx))
}

impl MacroDialog {
    fn new(modal: Modal, workbook: String, project: Option<Result<Rc<Project>, String>>, cx: &mut Context<Self>) -> Self {
        let (project, trouble) = match project {
            Some(Ok(project)) => (Some(project), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let macros = project.as_ref().map(|project| project.macros()).unwrap_or_default();
        // The first macro is proposed, as in Excel.
        let name = widgets::input(cx, macros.first().map(|m| m.name.clone()).unwrap_or_default());
        let subscription = cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                cx.notify();
            }
        });
        MacroDialog {
            modal,
            workbook,
            project,
            trouble,
            macros,
            name,
            scope: 0,
            _subscriptions: vec![subscription],
        }
    }

    /// The macro whose name is in the box.
    pub(crate) fn selected(&self, cx: &gpui::App) -> Option<usize> {
        let typed = self.name.read(cx).text().trim().to_lowercase();
        self.macros.iter().position(|m| m.name.to_lowercase() == typed)
    }

    pub(crate) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(found) = self.macros.get(index) {
            let name = found.name.clone();
            self.name.update(cx, |input, cx| input.set_text(name, cx));
            cx.notify();
        }
    }

    /// Run and Step Into: the one message, the dialog staying open.
    pub(crate) fn run(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        if self.selected(cx).is_some() {
            self.modal.update_app(cx, |app, cx| app.alert(not_run_alert(), cx));
        }
    }

    /// Edit: the module of the macro, read-only, the caret on the macro.
    pub(crate) fn edit(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (Some(index), Some(project)) = (self.selected(cx), self.project.clone()) else {
            return;
        };
        let chosen = self.macros[index].clone();
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            open_viewer(app, project, chosen.module, Some(chosen.name), window, cx);
        });
    }

    fn close(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
    }
}

impl Render for MacroDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected(cx);
        let chosen = selected.is_some();
        let names: Vec<SharedString> = self.macros.iter().map(|m| SharedString::from(m.name.clone())).collect();
        let scopes: Vec<SharedString> = vec![t("All Open Workbooks", "Tous les classeurs ouverts").into(), self.workbook.clone().into()];
        debug_assert_eq!(scopes.len(), SCOPES);
        let note = match (&self.trouble, self.project.is_some()) {
            (Some(trouble), _) => Some(format!(
                "{}\n{trouble}",
                t("The VBA project of this workbook can't be read.", "Impossible de lire le projet VBA de ce classeur.")
            )),
            (None, false) => Some(t("This workbook has no macros.", "Ce classeur ne contient aucune macro.").to_string()),
            (None, true) => None,
        };
        let buttons = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().debug_selector(|| "macro-run".to_string()).child(
                Button::new("macro-run", t("Run", "Exécuter")).width(104.0).primary().disabled(!chosen).on_click(cx.listener(Self::run)),
            ))
            .child(div().debug_selector(|| "macro-step".to_string()).child(
                Button::new("macro-step", t("Step Into", "Pas à pas détaillé")).width(104.0).disabled(!chosen).on_click(cx.listener(Self::run)),
            ))
            .child(div().debug_selector(|| "macro-edit".to_string()).child(
                Button::new("macro-edit", crate::i18n::t_ctx("macro", "Edit", "Modifier")).width(104.0).disabled(!chosen).on_click(cx.listener(Self::edit)),
            ))
            // These would change the project: never available.
            .child(div().debug_selector(|| "macro-create".to_string()).child(Button::new("macro-create", t("Create", "Créer")).width(104.0).disabled(true)))
            .child(div().debug_selector(|| "macro-delete".to_string()).child(Button::new("macro-delete", t("Delete", "Supprimer")).width(104.0).disabled(true)))
            .child(div().debug_selector(|| "macro-options".to_string()).child(Button::new("macro-options", t("Options...", "Options...")).width(104.0).disabled(true)))
            .child(div().h(px(6.)))
            .child(div().debug_selector(|| "macro-cancel".to_string()).child(
                Button::new("macro-cancel", t("Cancel", "Annuler")).width(104.0).on_click(cx.listener(Self::close)),
            ));
        let list = ListBox::new("macro-list", names, selected)
            .width(300.0)
            .height(190.0)
            .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx)))
            .on_activate(cx.listener(|this, index: &usize, window, cx| {
                this.select(*index, cx);
                this.edit(&(), window, cx);
            }));
        widgets::dialog(&self.modal, t("Macro", "Macro"))
            .width(450.0)
            .on_ok(cx.listener(Self::run))
            .on_cancel(cx.listener(Self::close))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(widgets::label(t("Macro name:", "Nom de la macro :")))
                            .child(div().flex().child(widgets::TextField::new(&self.name).width(300.0)))
                            .child(list)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .pt(px(4.))
                                    .child(widgets::label(t("Macros in:", "Macros dans :")))
                                    .child(
                                        Select::new("macro-scope", scopes, Some(self.scope))
                                            .width(200.0)
                                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                                this.scope = *index;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .children(note.map(|note| div().w(px(300.)).text_color(c(theme::TEXT_MUTED)).child(note))),
                    )
                    .child(buttons),
            )
    }
}

// -------------------------------------------------------------------- viewer

/// The modules of a project, read-only: their list, and the code of the
/// one chosen.
pub(crate) struct VbaViewer {
    modal: Modal,
    project: Rc<Project>,
    pub(crate) module: usize,
    pub(crate) editor: Entity<CodeEditor>,
}

/// The line (1-based) on which a macro of a module starts.
fn macro_line(code: &str, name: &str) -> Option<u32> {
    // "Sheet1.Refresh" is the macro Refresh of the module Sheet1.
    let name = name.rsplit('.').next().unwrap_or(name).to_lowercase();
    code.lines().position(|line| {
        // What follows a quote is a remark.
        let line = line.split('\'').next().unwrap_or(line);
        let words: Vec<String> = line.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
        words.iter().position(|w| w == "sub").is_some_and(|at| words.get(at + 1) == Some(&name) && words[..at].iter().all(|w| w == "public" || w == "static"))
    })
    .map(|index| index as u32 + 1)
}

pub(crate) fn open_viewer(
    app: &mut ExcelApp,
    project: Rc<Project>,
    module: usize,
    macro_name: Option<String>,
    window: &mut Window,
    cx: &mut Cx,
) -> Entity<VbaViewer> {
    app.open_modal(window, cx, |modal, _, cx| {
        let code = project.modules.get(module).map(|m| m.code()).unwrap_or_default();
        let line = macro_name.as_deref().and_then(|name| macro_line(&code, name));
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::viewer(&code, Language::Vba, cx);
            if let Some(line) = line {
                editor.go_to_line(line, None, cx);
            }
            editor
        });
        VbaViewer {
            modal,
            project,
            module,
            editor,
        }
    })
}

impl VbaViewer {
    pub(crate) fn show(&mut self, module: usize, cx: &mut Context<Self>) {
        // The module shown stays as it is, its caret where it was.
        if module == self.module {
            return;
        }
        if let Some(found) = self.project.modules.get(module) {
            self.module = module;
            let code = found.code();
            self.editor.update(cx, |editor, cx| editor.set_text(&code, cx));
            cx.notify();
        }
    }

    fn close(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
    }
}

/// A module in the list of the viewer: what kind it is, then its name.
fn module_label(kind: ModuleKind, name: &str) -> String {
    let kind = match kind {
        ModuleKind::Document => t("Object", "Objet"),
        ModuleKind::Standard => t("Module", "Module"),
        ModuleKind::Class => t("Class Module", "Module de classe"),
        ModuleKind::Form => t("UserForm", "UserForm"),
    };
    format!("{name}  —  {kind}")
}

impl Render for VbaViewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modules: Vec<SharedString> = self.project.modules.iter().map(|m| module_label(m.kind, &m.name).into()).collect();
        let title = format!("{} — {}", self.project.name, t("Read-Only", "Lecture seule"));
        widgets::dialog(&self.modal, title)
            .width(820.0)
            .on_ok(cx.listener(Self::close))
            .on_cancel(cx.listener(Self::close))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        ListBox::new("vba-modules", modules, Some(self.module))
                            .width(230.0)
                            .height(420.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| this.show(*index, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .h(px(420.))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .debug_selector(|| "vba-code".to_string())
                            .child(self.editor.clone()),
                    ),
            )
            .child(div().pt(px(8.)).text_color(c(theme::TEXT_MUTED)).child(t(
                "OpenSheet shows the VBA code of this workbook and keeps it in the file. It can't be changed or run here.",
                "OpenSheet affiche le code VBA de ce classeur et le conserve dans le fichier. Il ne peut être ni modifié ni exécuté ici.",
            )))
            .footer(div().flex().justify_end().child(Button::new("vba-close", t("Close", "Fermer")).primary().on_click(cx.listener(Self::close))))
    }
}

// ------------------------------------------------------------------- saving

/// The extension a new file of this workbook is proposed with: a
/// workbook that carries macros is a macro-enabled one.
pub(crate) fn default_extension(app: &ExcelApp) -> &'static str {
    if app.wb.has_vba_project() { "xlsm" } else { "xlsx" }
}

/// True when writing the workbook to `path` would leave its macros out:
/// a workbook file of a kind that cannot hold them.
pub(crate) fn loses_macros(app: &ExcelApp, path: &Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    app.wb.has_vba_project() && matches!(extension.as_str(), "xlsx" | "xltx")
}

/// Excel's question before a workbook with a VBA project is saved as a
/// macro-free one. "Yes" saves without the project (the open workbook
/// keeps it); "No", the default, saves nothing.
pub(crate) fn macro_free_alert(path: std::path::PathBuf) -> Alert {
    let message = format!(
        "{}\n\n{}\n\n{}\n\n{}",
        t(
            "The following features cannot be saved in macro-free workbooks:",
            "Les fonctionnalités suivantes ne peuvent pas être enregistrées dans des classeurs sans macro :"
        ),
        t("• VB project", "• Projet VB"),
        t(
            "To save a file with these features, click No, and then save the workbook with the .xlsm extension (Excel Macro-Enabled Workbook).",
            "Pour enregistrer un fichier avec ces fonctionnalités, cliquez sur Non, puis enregistrez le classeur avec l'extension .xlsm (Classeur Excel prenant en charge les macros)."
        ),
        t(
            "To continue saving as a macro-free workbook, click Yes.",
            "Pour continuer à enregistrer en tant que classeur sans macro, cliquez sur Oui."
        ),
    );
    Alert::new("OpenSheet", message)
        .warning()
        .button(t("Yes", "Oui"), move |app, _, cx| app.write_unasked(path.clone(), cx))
        .close_button(t("No", "Non"))
        .default_button(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macro_is_found_in_its_module() {
        let code = "Option Explicit\n\n' Sub Total() in a remark\nPrivate Sub Hidden()\nEnd Sub\n\nPublic Sub Total()\n    x = 1\nEnd Sub\n\nSub Refresh\nEnd Sub\n";
        assert_eq!(macro_line(code, "Total"), Some(7));
        assert_eq!(macro_line(code, "total"), Some(7));
        assert_eq!(macro_line(code, "Sheet1.Refresh"), Some(11));
        assert_eq!(macro_line(code, "Hidden"), None);
        assert_eq!(macro_line(code, "Missing"), None);
    }
}
