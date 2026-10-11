//! New / Open / Save.

use crate::app::ExcelApp;
use crate::i18n::t;
use gpui::{Context, Window};
use std::path::PathBuf;

impl ExcelApp {
    pub fn new_workbook(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let wb = opensheet_engine::Workbook::new(self.wb.locale);
        self.set_workbook(wb, None, format!("{}1", t("Book", "Classeur")), cx);
    }

    pub fn open(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.not_implemented(t("Open", "Ouvrir"), cx);
    }

    pub fn save(&mut self, _save_as: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.not_implemented(t("Save", "Enregistrer"), cx);
    }

    pub fn load_path(&mut self, _path: PathBuf, cx: &mut Context<Self>) {
        self.not_implemented(t("Open", "Ouvrir"), cx);
    }
}
