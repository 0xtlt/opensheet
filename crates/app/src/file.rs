//! New / Open / Save, and the "unsaved changes" guard.

use crate::app::{Dialog, ExcelApp};
use crate::i18n::t;
use gpui::{Context, PathPromptOptions, PromptLevel, Window};
use std::path::PathBuf;

fn file_title(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| t("Book", "Classeur").to_string())
}

impl ExcelApp {
    /// Run `then` right away when there is nothing to lose, otherwise ask
    /// first whether the unsaved changes may be discarded.
    pub fn confirm_discard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.wb.modified {
            then(self, window, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "{} « {} » ?",
                t(
                    "Do you want to discard the changes made to",
                    "Voulez-vous abandonner les modifications apportées à"
                ),
                self.title
            ),
            Some(t(
                "Your changes will be lost if you don't save them first.",
                "Vos modifications seront perdues si vous ne les enregistrez pas d'abord.",
            )),
            &[t("Cancel", "Annuler"), t("Discard", "Ne pas enregistrer")],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                this.update_in(cx, |this, window, cx| then(this, window, cx)).ok();
            }
        })
        .detach();
    }

    pub fn new_workbook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, |this, _, cx| {
            let wb = opensheet_engine::Workbook::new(this.wb.locale);
            this.set_workbook(wb, None, format!("{}1", t("Book", "Classeur")), cx);
        });
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, |_, _, cx| {
            let paths = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = paths.await {
                    if let Some(path) = paths.into_iter().next() {
                        this.update(cx, |this, cx| this.load_path(path, cx)).ok();
                    }
                }
            })
            .detach();
        });
    }

    pub fn load_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match opensheet_io::load(&path, self.wb.locale) {
            Ok(wb) => {
                let title = file_title(&path);
                self.set_workbook(wb, Some(path), title, cx);
            }
            Err(err) => {
                self.dialog = Some(Dialog {
                    title: t("Can't open this file", "Impossible d'ouvrir ce fichier").into(),
                    message: format!("{}\n\n{}", path.display(), err).into(),
                });
                cx.notify();
            }
        }
    }

    pub fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_editing() && !self.commit_edit(None, window, cx) {
            return;
        }
        if let (Some(path), false) = (self.path.clone(), save_as) {
            self.write_to(path, cx);
            return;
        }
        let dir = self
            .path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents")))
            .unwrap_or_else(|| PathBuf::from("."));
        let name = format!("{}.xlsx", self.title);
        let target = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut path))) = target.await {
                if path.extension().is_none() {
                    path.set_extension("xlsx");
                }
                this.update(cx, |this, cx| this.write_to(path, cx)).ok();
            }
        })
        .detach();
    }

    fn write_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match opensheet_io::save(&self.wb, self.sheet, &path) {
            Ok(()) => {
                // Only the workbook formats keep everything; a CSV export
                // leaves the document "edited".
                let lossless = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xlsx") || e.eq_ignore_ascii_case("xlsm"));
                if lossless {
                    self.wb.modified = false;
                    self.title = file_title(&path);
                    self.path = Some(path);
                }
                self.set_status(t("Saved", "Enregistré"), cx);
            }
            Err(err) => {
                self.dialog = Some(Dialog {
                    title: t("Can't save this file", "Impossible d'enregistrer ce fichier").into(),
                    message: format!("{}\n\n{}", path.display(), err).into(),
                });
                cx.notify();
            }
        }
    }

    /// Quit, after confirmation when there are unsaved changes.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, |_, _, cx| cx.quit());
    }
}
