//! The Print screen (File > Print, ⌘P): a preview of the pages as they
//! will print, beside the settings of the job. The preview paints the
//! scenes the PDF is written from; printing hands that PDF to the system
//! (`lp`), saves it, or opens it in Preview.

use super::paper::PaperPainter;
use super::{PageUi, change, scene, setup, show_breaks, views};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c, ca};
use crate::widgets::{self, Button, Checkbox, NumberField, ScrollArea, Select, dialog, label};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FontWeight, Pixels, Render, SharedString, Window, canvas, div, fill, point,
    prelude::*, px, size,
};
use opensheet_engine::page::{MarginPreset, Margins, Orientation, PageSetup, PaperSize};
use opensheet_engine::{Locale, Rect};
use opensheet_print::{PrintJob, PrintPlan, PrintWhat, plan, print_pdf, render_page};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

/// What the Print screen remembers from one opening to the next.
#[derive(Default)]
pub(crate) struct Memory {
    printer: Option<String>,
    show_margins: bool,
}

/// The printers `lpstat -p -d` lists (run in the C locale), and the
/// default one.
pub(crate) fn parse_printers(output: &str) -> (Vec<String>, Option<String>) {
    let mut printers = Vec::new();
    let mut default = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("printer ") {
            if let Some(name) = rest.split_whitespace().next() {
                printers.push(name.to_string());
            }
        } else if let Some(name) = line.strip_prefix("system default destination: ") {
            default = Some(name.trim().to_string());
        }
    }
    (printers, default)
}

fn printers() -> (Vec<String>, Option<String>) {
    Command::new("lpstat")
        .args(["-p", "-d"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .map(|out| parse_printers(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// The arguments of `lp` for a job.
pub(crate) fn lp_arguments(printer: &str, copies: u32, collate: bool, title: &str, file: &str) -> Vec<String> {
    let mut args = vec!["-d".to_string(), printer.to_string(), "-n".to_string(), copies.max(1).to_string()];
    if copies > 1 {
        args.extend(["-o".to_string(), format!("collate={collate}")]);
    }
    args.extend(["-t".to_string(), title.to_string(), file.to_string()]);
    args
}

/// The four ways of the Scaling list, as an index: no scaling, the sheet
/// on one page, all columns on one page, all rows on one page. `None`
/// for any other "Fit to".
pub(crate) fn scaling_index(setup: &PageSetup) -> Option<usize> {
    match (setup.fit_to_page, setup.fit_width, setup.fit_height) {
        (false, _, _) => Some(0),
        (true, 1, 1) => Some(1),
        (true, 1, 0) => Some(2),
        (true, 0, 1) => Some(3),
        _ => None,
    }
}

pub(crate) fn apply_scaling(setup: &mut PageSetup, index: usize) {
    let (fit, width, height) = match index {
        1 => (true, 1, 1),
        2 => (true, 1, 0),
        3 => (true, 0, 1),
        _ => (false, setup.fit_width, setup.fit_height),
    };
    setup.fit_to_page = fit;
    (setup.fit_width, setup.fit_height) = (width, height);
}

/// A file name without what a file system refuses.
fn file_stem(title: &str) -> String {
    let clean: String = title.chars().map(|ch| if matches!(ch, '/' | ':' | '\\' | '\0') { '-' } else { ch }).collect();
    let clean = clean.trim().trim_end_matches(".xlsx").to_string();
    if clean.is_empty() { "OpenSheet".to_string() } else { clean }
}

/// What the preview shows of.
#[derive(Clone, PartialEq)]
struct Key {
    generation: u64,
    sheet: usize,
    what: usize,
    ignore_area: bool,
    page: usize,
    selection: Vec<Rect>,
}

struct Preview {
    key: Key,
    plan: PrintPlan,
    scene: Option<Rc<scene::Prepared>>,
    /// The margins of the page shown: left, top, right, bottom, header
    /// and footer, in points.
    margins: [f64; 6],
}

struct PrintDialog {
    modal: Modal,
    painter: Rc<PaperPainter>,
    printers: Vec<String>,
    /// Index into `printers`; one past the end is "Save as PDF".
    target: usize,
    copies: Entity<TextInput>,
    /// Active sheets, entire workbook or selection.
    what: usize,
    ignore_area: bool,
    from: Entity<TextInput>,
    to: Entity<TextInput>,
    collate: bool,
    /// The page shown, from 0.
    page: usize,
    show_margins: bool,
    /// The whole page fits the preview ("Zoom to Page" off shows it at
    /// its real size).
    whole_page: bool,
    preview: Option<Preview>,
}

fn job<'a>(app: &ExcelApp, what: usize, ignore_area: bool, painter: &'a PaperPainter) -> PrintJob<'a> {
    let what = match what {
        1 => PrintWhat::Workbook,
        2 => PrintWhat::Selection {
            sheet: app.sheet,
            ranges: app.sel().ranges.clone(),
        },
        _ => PrintWhat::Sheet(app.sheet),
    };
    let mut job: PrintJob<'a> = PrintJob::new(what);
    (job.file_name, job.file_path) = views::file_names(app);
    job.author = views::user_name();
    job.ignore_print_area = ignore_area;
    job.painter = Some(painter);
    job
}

/// Open the Print screen on the current sheet.
pub(crate) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    // A sheet that was previewed shows its page breaks from then on.
    show_breaks(app, app.sheet);
    let (printers, default) = printers();
    let memory = &app.ext_mut::<PageUi>().print;
    let wanted = memory.printer.clone().or(default);
    let target = wanted
        .and_then(|name| printers.iter().position(|p| *p == name))
        .unwrap_or(0);
    let show_margins = memory.show_margins;
    let painter = Rc::new(PaperPainter::new(cx));
    app.open_modal(window, cx, move |modal, _, cx| PrintDialog {
        modal,
        painter,
        printers,
        target,
        copies: widgets::number_input(cx, Some(1.0), 1.0, 999.0, 1.0),
        what: 0,
        ignore_area: false,
        from: widgets::number_input(cx, None, 1.0, 9999.0, 1.0),
        to: widgets::number_input(cx, None, 1.0, 9999.0, 1.0),
        collate: true,
        page: 0,
        show_margins,
        whole_page: true,
        preview: None,
    });
}

impl PrintDialog {
    /// Bring the preview up to date with the workbook and the settings.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let painter = self.painter.clone();
        let (what, ignore_area, page) = (self.what, self.ignore_area, self.page);
        let current = self.preview.as_ref().map(|p| p.key.clone());
        let fresh = self.modal.read_app(cx, |app| {
            let key = Key {
                generation: app.wb.generation,
                sheet: app.sheet,
                what,
                ignore_area,
                page,
                selection: if what == 2 { app.sel().ranges.clone() } else { Vec::new() },
            };
            if current.as_ref() == Some(&key) {
                return None;
            }
            let job = job(app, what, ignore_area, &painter);
            let plan = plan(&app.wb, &job);
            let page = page.min(plan.pages.len().saturating_sub(1));
            let scene = render_page(&app.wb, &job, &plan, page);
            let sheet = plan.sheet_of(page).unwrap_or(app.sheet);
            let m = &app.wb.sheets[sheet].page.margins;
            let margins = [m.left, m.top, m.right, m.bottom, m.header, m.footer].map(|inches| inches * 72.0);
            Some((Key { page, ..key }, plan, scene, margins))
        });
        if let Some(Some((key, plan, scene, margins))) = fresh {
            self.page = key.page;
            let scene = scene.map(|scene| Rc::new(scene::prepare(scene, cx)));
            self.preview = Some(Preview {
                key,
                plan,
                scene,
                margins,
            });
        }
    }

    fn page_count(&self) -> usize {
        self.preview.as_ref().map_or(0, |p| p.plan.pages.len())
    }

    /// The page range typed in "Pages", if any.
    fn page_range(&self, cx: &App) -> Option<(u32, u32)> {
        let from = self.from.read(cx).number().map(|n| n.max(1.0) as u32);
        let to = self.to.read(cx).number().map(|n| n.max(1.0) as u32);
        match (from, to) {
            (None, None) => None,
            (from, to) => Some((from.unwrap_or(1), to.unwrap_or(u32::MAX))),
        }
    }

    fn copies(&self, cx: &App) -> u32 {
        self.copies.read(cx).number().map_or(1, |n| n.clamp(1.0, 999.0) as u32)
    }

    /// The PDF of the job and the title it prints under; `None` (after
    /// telling the user) when there is nothing to print.
    fn pdf(&mut self, cx: &mut Context<Self>) -> Option<(Vec<u8>, String)> {
        let painter = self.painter.clone();
        let (what, ignore_area) = (self.what, self.ignore_area);
        let pages = self.page_range(cx);
        let made = self.modal.read_app(cx, |app| {
            let mut job = job(app, what, ignore_area, &painter);
            job.pages = pages;
            if plan(&app.wb, &job).output.is_empty() {
                return None;
            }
            Some((print_pdf(&app.wb, &job), app.title.clone()))
        })?;
        if made.is_none() {
            self.tell(
                t("We didn't find anything to print.", "Désolé... Nous n'avons trouvé aucun contenu à imprimer.").into(),
                cx,
            );
        }
        made
    }

    fn tell(&self, message: SharedString, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
    }

    /// Write the PDF where the system's tools can read it.
    fn temp_file(&mut self, cx: &mut Context<Self>) -> Option<(PathBuf, String)> {
        let (pdf, title) = self.pdf(cx)?;
        let dir = std::env::temp_dir().join("OpenSheet");
        let path = dir.join(format!("{}.pdf", file_stem(&title)));
        let written = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, pdf));
        match written {
            Ok(()) => Some((path, title)),
            Err(err) => {
                self.tell(format!("{}\n\n{err}", path.display()).into(), cx);
                None
            }
        }
    }

    fn remember(&self, cx: &mut Context<Self>) {
        let printer = self.printers.get(self.target).cloned();
        let show_margins = self.show_margins;
        self.modal.update_app(cx, |app, _| {
            let memory = &mut app.ext_mut::<PageUi>().print;
            memory.printer = printer;
            memory.show_margins = show_margins;
        });
    }

    fn finish(&mut self, status: String, window: &mut Window, cx: &mut Context<Self>) {
        self.remember(cx);
        self.modal.update_app(cx, |app, cx| app.set_status(status, cx));
        self.modal.close(window, cx);
    }

    /// The Print button: send the job to the chosen printer, or save it.
    fn print(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(printer) = self.printers.get(self.target).cloned() else {
            self.save_pdf(&(), window, cx);
            return;
        };
        let (copies, collate) = (self.copies(cx), self.collate);
        let Some((path, title)) = self.temp_file(cx) else {
            return;
        };
        let arguments = lp_arguments(&printer, copies, collate, &title, &path.to_string_lossy());
        match Command::new("lp").args(&arguments).output() {
            Ok(out) if out.status.success() => {
                let status = format!("{} {printer}", t("Sent to printer", "Envoyé à l'imprimante"));
                self.finish(status, window, cx);
            }
            Ok(out) => self.tell(String::from_utf8_lossy(&out.stderr).trim().to_string().into(), cx),
            Err(err) => self.tell(err.to_string().into(), cx),
        }
    }

    fn save_pdf(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some((pdf, title)) = self.pdf(cx) else {
            return;
        };
        let dir = self
            .modal
            .read_app(cx, |app| app.path.as_ref().and_then(|p| p.parent().map(PathBuf::from)))
            .flatten()
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents")))
            .unwrap_or_else(|| PathBuf::from("."));
        let target = cx.prompt_for_new_path(&dir, Some(&format!("{}.pdf", file_stem(&title))));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut path))) = target.await else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension("pdf");
            }
            this.update_in(cx, |this, window, cx| match std::fs::write(&path, &pdf) {
                Ok(()) => {
                    let status = format!("{} {}", t("Saved as PDF:", "Enregistré au format PDF :"), path.display());
                    this.finish(status, window, cx);
                }
                Err(err) => this.tell(format!("{}\n\n{err}", path.display()).into(), cx),
            })
            .ok();
        })
        .detach();
    }

    fn open_in_preview(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        let Some((path, _)) = self.temp_file(cx) else {
            return;
        };
        if let Err(err) = Command::new("open").args(["-a", "Preview"]).arg(&path).spawn() {
            self.tell(err.to_string().into(), cx);
        }
        self.remember(cx);
    }

    /// Change the page setup of the sheet shown: the preview follows.
    fn set(&mut self, label: &'static str, cx: &mut Context<Self>, f: impl FnOnce(&mut PageSetup) + 'static) {
        self.modal.update_app(cx, |app, cx| {
            change(app, label, cx, f);
        });
        cx.notify();
    }

    fn step(&mut self, delta: i64, cx: &mut Context<Self>) {
        let last = self.page_count().saturating_sub(1) as i64;
        self.page = (self.page as i64 + delta).clamp(0, last.max(0)) as usize;
        cx.notify();
    }

    // ------------------------------------------------------------- drawing

    fn settings(&mut self, setup: &PageSetup, locale: &'static Locale, last_margins: Option<Margins>, cx: &mut Context<Self>) -> AnyElement {
        let names = |list: &[&'static str]| -> Vec<SharedString> { list.iter().map(|name| (*name).into()).collect() };
        let heading = |text: &'static str| div().pt(px(4.)).font_weight(FontWeight::SEMIBOLD).text_size(px(13.)).child(text);
        let wide = 290.0;

        let mut targets: Vec<SharedString> = self.printers.iter().map(|name| name.replace('_', " ").into()).collect();
        targets.push(t("Save as PDF", "Enregistrer au format PDF").into());

        let paper = setup.paper_size(locale);
        let mut papers: Vec<PaperSize> = PaperSize::COMMON.to_vec();
        if !papers.contains(&paper) {
            papers.push(paper);
        }
        let paper_names: Vec<SharedString> = papers
            .iter()
            .map(|p| format!("{}  ({})", p.name(locale), p.dimensions_text(locale)).into())
            .collect();

        // Margins: the three presets, the last custom ones, then whatever
        // the sheet has when it is none of these.
        let mut margin_names = names(&[
            t("Normal Margins", "Marges normales"),
            t("Wide Margins", "Marges larges"),
            t("Narrow Margins", "Marges étroites"),
        ]);
        let mut margin_values = vec![Some(Margins::NORMAL), Some(Margins::WIDE), Some(Margins::NARROW)];
        if let Some(last) = last_margins.filter(|m| m.preset().is_none()) {
            margin_names.push(t("Last Custom Margins Setting", "Dernier paramètre de marges personnalisé").into());
            margin_values.push(Some(last));
        }
        let mut margin_index = match setup.margins.preset() {
            Some(MarginPreset::Normal) => Some(0),
            Some(MarginPreset::Wide) => Some(1),
            Some(MarginPreset::Narrow) => Some(2),
            None => margin_values.iter().position(|m| *m == Some(setup.margins)),
        };
        if margin_index.is_none() {
            margin_names.push(t("Custom Margins", "Marges personnalisées").into());
            margin_values.push(Some(setup.margins));
            margin_index = Some(margin_names.len() - 1);
        }
        margin_names.push(t("Custom Margins…", "Marges personnalisées…").into());
        margin_values.push(None);

        let mut scalings = names(&[
            t("No Scaling", "Pas de mise à l'échelle"),
            t("Fit Sheet on One Page", "Ajuster la feuille à une page"),
            t("Fit All Columns on One Page", "Ajuster toutes les colonnes à une page"),
            t("Fit All Rows on One Page", "Ajuster toutes les lignes à une page"),
        ]);
        let scaling = scaling_index(setup).unwrap_or_else(|| {
            scalings.push(t("Custom Scaling", "Mise à l'échelle personnalisée").into());
            4
        });
        scalings.push(t("Custom Scaling Options…", "Options de mise à l'échelle personnalisée…").into());
        let scaling_options = scalings.len() - 1;

        let nothing = self.preview.as_ref().is_some_and(|p| p.plan.pages.is_empty());
        div()
            .w(px(wide + 10.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        Button::new("print", t("Print", "Imprimer"))
                            .primary()
                            .width(96.0)
                            .disabled(nothing)
                            .on_click(cx.listener(Self::print)),
                    )
                    .child(label(t("Copies:", "Copies :")))
                    .child(NumberField::new("copies", &self.copies).width(60.0)),
            )
            .child(heading(t("Printer", "Imprimante")))
            .child(
                Select::new("target", targets, Some(self.target.min(self.printers.len())))
                    .width(wide)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.target = *ix;
                        cx.notify();
                    })),
            )
            .child(heading(t("Settings", "Paramètres")))
            .child(
                Select::new(
                    "what",
                    names(&[
                        t("Print Active Sheets", "Imprimer les feuilles actives"),
                        t("Print Entire Workbook", "Imprimer le classeur entier"),
                        t("Print Selection", "Imprimer la sélection"),
                    ]),
                    Some(self.what),
                )
                .width(wide)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.what = *ix;
                    this.page = 0;
                    cx.notify();
                })),
            )
            .child(
                Checkbox::new("ignore-area", t("Ignore Print Area", "Ignorer la zone d'impression"), self.ignore_area).on_change(
                    cx.listener(|this, on: &bool, _, cx| {
                        this.ignore_area = *on;
                        this.page = 0;
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Pages:", "Pages :")))
                    .child(NumberField::new("from", &self.from).width(64.0))
                    .child(label(t("to", "à")))
                    .child(NumberField::new("to", &self.to).width(64.0)),
            )
            .child(
                Select::new(
                    "collate",
                    names(&[
                        t("Collated  1,2,3  1,2,3  1,2,3", "Assemblées  1,2,3  1,2,3  1,2,3"),
                        t("Uncollated  1,1,1  2,2,2  3,3,3", "Non assemblées  1,1,1  2,2,2  3,3,3"),
                    ]),
                    Some(if self.collate { 0 } else { 1 }),
                )
                .width(wide)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.collate = *ix == 0;
                    cx.notify();
                })),
            )
            .child(
                Select::new(
                    "orientation",
                    names(&[
                        t("Portrait Orientation", "Orientation Portrait"),
                        t("Landscape Orientation", "Orientation Paysage"),
                    ]),
                    Some(if setup.orientation == Orientation::Portrait { 0 } else { 1 }),
                )
                .width(wide)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    let orientation = if *ix == 0 { Orientation::Portrait } else { Orientation::Landscape };
                    this.set(t("Orientation", "Orientation"), cx, move |s| s.orientation = orientation);
                })),
            )
            .child(
                Select::new("paper", paper_names, papers.iter().position(|p| *p == paper))
                    .width(wide)
                    .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                        if let Some(paper) = papers.get(*ix).copied() {
                            this.set(t("Paper Size", "Format du papier"), cx, move |s| s.paper = Some(paper));
                        }
                    })),
            )
            .child(
                Select::new("margins", margin_names, margin_index)
                    .width(wide)
                    .on_change(cx.listener(move |this, ix: &usize, window, cx| match margin_values.get(*ix).copied().flatten() {
                        Some(margins) => this.set(t("Margins", "Marges"), cx, move |s| s.margins = margins),
                        None => this.open_setup(setup::TAB_MARGINS, window, cx),
                    })),
            )
            .child(
                Select::new("scaling", scalings, Some(scaling))
                    .width(wide)
                    .on_change(cx.listener(move |this, ix: &usize, window, cx| {
                        let index = *ix;
                        if index == scaling_options {
                            this.open_setup(setup::TAB_PAGE, window, cx);
                        } else if index < 4 {
                            this.set(t("Scale to Fit", "Mise à l'échelle"), cx, move |s| apply_scaling(s, index));
                        }
                    })),
            )
            .child(
                div()
                    .id("page-setup-link")
                    .text_color(c(0x0F6CBD))
                    .hover(|s| s.underline())
                    .child(t("Page Setup", "Mise en page"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_setup(setup::TAB_PAGE, window, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("open-preview", t("Open in Preview", "Ouvrir dans Aperçu"))
                            .disabled(nothing)
                            .on_click(cx.listener(Self::open_in_preview)),
                    )
                    .child(
                        Button::new("save-pdf", t("Save as PDF…", "Enregistrer au format PDF…"))
                            .disabled(nothing)
                            .on_click(cx.listener(Self::save_pdf)),
                    ),
            )
            .into_any_element()
    }

    /// Page Setup opens over the Print screen, which follows what it
    /// changes.
    fn open_setup(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| setup::open(app, tab, window, cx));
    }

    /// The page shown, on the grey of the preview.
    fn page_view(&self) -> AnyElement {
        const AREA: (f32, f32) = (640.0, 590.0);
        let area = div()
            .w(px(AREA.0))
            .h(px(AREA.1))
            .flex_none()
            .bg(c(0xE6E6E6))
            .border_1()
            .border_color(c(theme::BORDER))
            .overflow_hidden();
        let Some(preview) = &self.preview else {
            return area.into_any_element();
        };
        let (Some(scene), Some(paper)) = (preview.scene.clone(), preview.plan.paper(self.page)) else {
            return area
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(theme::TEXT_MUTED))
                .child(t("We didn't find anything to print.", "Désolé... Nous n'avons trouvé aucun contenu à imprimer."))
                .into_any_element();
        };
        // The whole page in the area, or the page at its real size.
        let fit = ((AREA.0 - 32.0) / paper.0 as f32).min((AREA.1 - 32.0) / paper.1 as f32);
        let scale = if self.whole_page { fit } else { 96.0 / 72.0 };
        let (w, h) = (paper.0 as f32 * scale, paper.1 as f32 * scale);
        let margins = self.show_margins.then_some(preview.margins);
        let sheet = canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window, cx| {
                // The paper and its shadow.
                window.paint_quad(fill(Bounds::new(bounds.origin + point(px(2.), px(2.)), bounds.size), ca(0x000000, 0.18)));
                window.paint_quad(fill(bounds, c(0xFFFFFF)));
                scene::paint(&scene, bounds, window, cx);
                let Some(m) = margins else {
                    return;
                };
                // Margins as dashed lines across the page.
                let k = f32::from(bounds.size.width) / paper.0 as f32;
                let ink = ca(0x000000, 0.55);
                let dashes = |window: &mut Window, vertical: bool, at: f32| {
                    let length = f32::from(if vertical { bounds.size.height } else { bounds.size.width });
                    let mut done = 0.0;
                    while done < length {
                        let piece = 4.0f32.min(length - done);
                        let (origin, extent) = if vertical {
                            (bounds.origin + point(px(at), px(done)), size(px(1.), px(piece)))
                        } else {
                            (bounds.origin + point(px(done), px(at)), size(px(piece), px(1.)))
                        };
                        window.paint_quad(fill(Bounds::new(origin, extent), ink));
                        done += 7.0;
                    }
                };
                let (pw, ph) = (paper.0 as f32 * k, paper.1 as f32 * k);
                dashes(window, true, m[0] as f32 * k);
                dashes(window, true, pw - m[2] as f32 * k);
                dashes(window, false, m[1] as f32 * k);
                dashes(window, false, ph - m[3] as f32 * k);
                dashes(window, false, m[4] as f32 * k);
                dashes(window, false, ph - m[5] as f32 * k);
            },
        )
        .w(px(w))
        .h(px(h))
        .flex_none();
        if self.whole_page {
            area.flex().items_center().justify_center().child(sheet).into_any_element()
        } else {
            area.child(ScrollArea::new("page-scroll", AREA.1 - 2.0).child(div().p(px(16.)).child(sheet)))
                .into_any_element()
        }
    }

    /// Under the page: which page is shown, and how.
    fn page_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.page_count();
        let toggle = |id: &'static str, text: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .when(!on, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(text)
        };
        let arrow = |id: &'static str, icon_name: &'static str, enabled: bool| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(widgets::icon(icon_name, 12.0, if enabled { theme::TEXT } else { theme::TEXT_DISABLED }))
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(arrow("page-prev", "chevron-left", self.page > 0).on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))))
            .child(
                div()
                    .min_w(px(34.))
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .rounded(px(3.))
                    .child(if count == 0 { "0".to_string() } else { (self.page + 1).to_string() }),
            )
            .child(format!("{} {count}", t("of", "sur")))
            .child(
                arrow("page-next", "chevron-right", self.page + 1 < count)
                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
            )
            .child(div().flex_1())
            .child(
                toggle("show-margins", t("Show Margins", "Afficher les marges"), self.show_margins).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.show_margins = !this.show_margins;
                        cx.notify();
                    },
                )),
            )
            .child(
                toggle("zoom-page", t("Zoom to Page", "Zoom sur la page"), !self.whole_page).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.whole_page = !this.whole_page;
                        cx.notify();
                    },
                )),
            )
            .into_any_element()
    }
}

impl Render for PrintDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        let state = self.modal.read_app(cx, |app| {
            // The settings are those of the sheet whose page is shown.
            let sheet = self.preview.as_ref().and_then(|p| p.plan.sheet_of(self.page)).unwrap_or(app.sheet);
            let sheet = sheet.min(app.wb.sheets.len() - 1);
            (
                app.wb.sheets[sheet].page.clone(),
                app.wb.locale,
                app.ext::<PageUi>().and_then(|ui| ui.last_custom_margins),
            )
        });
        let Some((setup, locale, last_margins)) = state else {
            return dialog(&self.modal, t("Print", "Imprimer"));
        };
        let settings = self.settings(&setup, locale, last_margins, cx);
        let page = self.page_view();
        let bar = self.page_bar(cx);
        dialog(&self.modal, t("Print", "Imprimer"))
            .width(1000.0)
            .on_ok(cx.listener(Self::print))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(settings)
                    .child(div().flex().flex_col().gap(px(8.)).child(page).child(bar)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_are_read_from_lpstat() {
        let output = "printer HP_LaserJet_Pro is idle.  enabled since Mon Oct 12 09:00:00 2026\n\
                      \tReady to print.\n\
                      printer Canon_TS5000 disabled since Sun Oct 11 18:00:00 2026 -\n\
                      \treason unknown\n\
                      system default destination: HP_LaserJet_Pro\n";
        let (printers, default) = parse_printers(output);
        assert_eq!(printers, vec!["HP_LaserJet_Pro", "Canon_TS5000"]);
        assert_eq!(default.as_deref(), Some("HP_LaserJet_Pro"));
        assert_eq!(parse_printers("no system default destination\n"), (Vec::new(), None));
    }

    #[test]
    fn lp_gets_the_job() {
        assert_eq!(
            lp_arguments("HP_LaserJet_Pro", 1, true, "Budget", "/tmp/Budget.pdf"),
            vec!["-d", "HP_LaserJet_Pro", "-n", "1", "-t", "Budget", "/tmp/Budget.pdf"]
        );
        assert_eq!(
            lp_arguments("P", 3, false, "B", "f.pdf"),
            vec!["-d", "P", "-n", "3", "-o", "collate=false", "-t", "B", "f.pdf"]
        );
    }

    #[test]
    fn scaling_presets() {
        let mut setup = PageSetup::default();
        assert_eq!(scaling_index(&setup), Some(0));
        apply_scaling(&mut setup, 2);
        assert_eq!((setup.fit_to_page, setup.fit_width, setup.fit_height), (true, 1, 0));
        assert_eq!(scaling_index(&setup), Some(2));
        apply_scaling(&mut setup, 3);
        assert_eq!(scaling_index(&setup), Some(3));
        apply_scaling(&mut setup, 1);
        assert_eq!(scaling_index(&setup), Some(1));
        // Back to "No Scaling": the percentage is in force again.
        apply_scaling(&mut setup, 0);
        assert!(!setup.fit_to_page);
        // Two pages wide is none of the four.
        setup.fit_to_page = true;
        setup.fit_width = 2;
        assert_eq!(scaling_index(&setup), None);
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(file_stem("Budget 2026"), "Budget 2026");
        assert_eq!(file_stem("a/b:c"), "a-b-c");
        assert_eq!(file_stem("  "), "OpenSheet");
        assert_eq!(file_stem("Ventes.xlsx"), "Ventes");
    }
}
