//! Lists: list box, check-list with search, tree with tri-state checks.
//! All three build only the rows in view, so they stay quick with tens of
//! thousands of items.

use super::{
    CONTROL_H, DropState, FIELD_BORDER, Handler, ROW_H, SELECTED_BG, ScrollState, TextField, Tri, check_mark, focus_of, icon,
    plain, virtual_rows,
};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use gpui::{
    AnyElement, App, ClickEvent, ElementId, Entity, IntoElement, KeyDownEvent, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};
use std::rc::Rc;

/// The keyboard cursor of a list and the selection it last scrolled to.
struct ListCursor {
    /// Row the arrows start from.
    cursor: usize,
    /// Fixed end of a Shift selection.
    anchor: usize,
    /// Selected row last brought into view.
    revealed: Option<usize>,
}

impl ListCursor {
    fn new() -> ListCursor {
        ListCursor {
            cursor: 0,
            anchor: 0,
            revealed: None,
        }
    }
}

/// The row a navigation key leads to from `from`, among `count` rows of
/// which `page` fit in the box.
fn navigate(key: &str, from: Option<usize>, count: usize, page: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    Some(match (key, from) {
        ("up", Some(i)) => i.saturating_sub(1),
        ("down", Some(i)) => (i + 1).min(last),
        ("pageup", Some(i)) => i.saturating_sub(page.max(1)),
        ("pagedown", Some(i)) => (i + page.max(1)).min(last),
        ("home", _) | ("up", None) | ("pageup", None) => 0,
        ("end", _) => last,
        ("down", None) | ("pagedown", None) => 0,
        _ => return None,
    })
}

/// The frame shared by the three lists: `width` pixels wide, or as wide
/// as the room left in its row.
fn list_frame(focused: bool, width: Option<f32>) -> gpui::Div {
    div()
        .when_some(width, |d, w| d.w(px(w)).flex_none())
        .when(width.is_none(), |d| d.flex_grow().min_w(px(60.)))
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
        .rounded(px(3.))
        .overflow_hidden()
}

// ----------------------------------------------------------------- list box

/// A list of choices, one of them selected (or several with
/// [`ListBox::multi`]).
///
/// Up / Down / Home / End / Page Up / Page Down move the selection, typing
/// letters jumps to the first item starting with them. In a multiple list,
/// Shift extends the selection and Cmd-click toggles one item.
///
/// ```ignore
/// ListBox::new("category", categories, Some(self.category))
///     .width(140.0)
///     .height(200.0)
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.category = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct ListBox {
    id: ElementId,
    items: Vec<SharedString>,
    selected: Vec<usize>,
    multi: bool,
    width: Option<f32>,
    height: f32,
    row: Option<Rc<dyn Fn(usize, bool) -> AnyElement>>,
    on_change: Option<Handler<usize>>,
    on_change_multi: Option<Handler<Vec<usize>>>,
    on_activate: Option<Handler<usize>>,
}

impl ListBox {
    /// `selected`: `None` shows no selection (the cells disagree).
    pub fn new(id: impl Into<ElementId>, items: Vec<SharedString>, selected: Option<usize>) -> ListBox {
        ListBox {
            id: id.into(),
            items,
            selected: selected.into_iter().collect(),
            multi: false,
            width: None,
            height: ROW_H * 8.0,
            row: None,
            on_change: None,
            on_change_multi: None,
            on_activate: None,
        }
    }

    /// A list in which several items can be selected.
    pub fn multi(id: impl Into<ElementId>, items: Vec<SharedString>, selected: Vec<usize>) -> ListBox {
        let mut list = ListBox::new(id, items, None);
        list.selected = selected;
        list.selected.sort_unstable();
        list.multi = true;
        list
    }

    /// Fixed width; without it the list takes the room left in its row.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Draw the rows yourself (a line style, a sample in red): the function
    /// gets the index of the item and whether it is selected.
    pub fn row(mut self, f: impl Fn(usize, bool) -> AnyElement + 'static) -> Self {
        self.row = Some(Rc::new(f));
        self
    }

    /// The selected item changed (single lists).
    pub fn on_change(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// The set of selected items changed (multiple lists), sorted.
    pub fn on_change_multi(mut self, f: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> Self {
        self.on_change_multi = Some(Rc::new(f));
        self
    }

    /// An item was double-clicked.
    pub fn on_activate(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_activate = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for ListBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollState::new());
        let cursor = window.use_keyed_state(self.id.clone(), cx, |_, _| ListCursor::new());
        let typing = window.use_keyed_state(self.id.clone(), cx, |_, _| DropState::new());
        let items = Rc::new(self.items);
        let count = items.len();
        let height = self.height;
        let page = (height / ROW_H) as usize;
        let multi = self.multi;
        let selected = Rc::new(self.selected);
        let lead = selected.first().copied();

        // A selection made elsewhere (another control, the first display)
        // is brought into view once.
        if cursor.read(cx).revealed != lead {
            cursor.update(cx, |s, _| {
                s.revealed = lead;
                if let Some(lead) = lead {
                    s.cursor = lead;
                    s.anchor = lead;
                }
            });
            if let Some(lead) = lead {
                scroll.update(cx, |s, _| s.reveal(lead, lead, ROW_H, height));
            }
        }

        // Every way of changing the selection ends here.
        let select: Rc<dyn Fn(Vec<usize>, usize, &mut Window, &mut App)> = {
            let (on_change, on_change_multi) = (self.on_change.clone(), self.on_change_multi.clone());
            let (cursor, scroll) = (cursor.clone(), scroll.clone());
            Rc::new(move |mut rows: Vec<usize>, at: usize, window: &mut Window, cx: &mut App| {
                rows.sort_unstable();
                rows.dedup();
                cursor.update(cx, |s, _| {
                    s.cursor = at;
                    s.revealed = rows.first().copied();
                });
                scroll.update(cx, |s, cx| {
                    s.reveal(at, at, ROW_H, height);
                    cx.notify();
                });
                if multi {
                    if let Some(f) = &on_change_multi {
                        f(&rows, window, cx);
                    }
                } else if let (Some(f), Some(row)) = (&on_change, rows.first()) {
                    f(row, window, cx);
                }
            })
        };

        let row_fn = self.row.clone();
        let on_activate = self.on_activate.clone();
        let rows = virtual_rows(&scroll, count, ROW_H, height, cx, |i| {
            let on = selected.binary_search(&i).is_ok();
            let select = select.clone();
            let selected = selected.clone();
            let cursor = cursor.clone();
            let on_activate = on_activate.clone();
            let content = match &row_fn {
                Some(f) => f(i, on),
                None => div().truncate().child(items[i].clone()).into_any_element(),
            };
            div()
                .id(("row", i))
                .size_full()
                .px(px(6.))
                .flex()
                .items_center()
                .whitespace_nowrap()
                .when(on, |d| d.bg(c(if focused { SELECTED_BG } else { theme::HOVER })))
                .when(!on, |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .child(content)
                .on_click(move |ev: &ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        if let Some(f) = &on_activate {
                            f(&i, window, cx);
                        }
                        return;
                    }
                    let modifiers = ev.modifiers();
                    let rows = if multi && modifiers.shift {
                        let anchor = cursor.read(cx).anchor;
                        (anchor.min(i)..=anchor.max(i)).collect()
                    } else if multi && (modifiers.platform || modifiers.control) {
                        let mut rows = (*selected).clone();
                        match rows.binary_search(&i) {
                            Ok(at) => {
                                rows.remove(at);
                            }
                            Err(at) => rows.insert(at, i),
                        }
                        cursor.update(cx, |s, _| s.anchor = i);
                        rows
                    } else {
                        cursor.update(cx, |s, _| s.anchor = i);
                        vec![i]
                    };
                    select(rows, i, window, cx);
                })
                .into_any_element()
        });

        let key_select = select.clone();
        let key_cursor = cursor.clone();
        let key_items = items.clone();
        list_frame(focused, self.width)
            .id(self.id)
            .track_focus(&focus)
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if count == 0 || !plain(ev) {
                    return;
                }
                let key = ev.keystroke.key.as_str();
                let shift = ev.keystroke.modifiers.shift;
                let at = Some(key_cursor.read(cx).cursor).filter(|_| lead.is_some() || multi);
                if let Some(next) = navigate(key, at, count, page) {
                    let rows = if multi && shift {
                        let anchor = key_cursor.read(cx).anchor;
                        (anchor.min(next)..=anchor.max(next)).collect()
                    } else {
                        key_cursor.update(cx, |s, _| s.anchor = next);
                        vec![next]
                    };
                    key_select(rows, next, window, cx);
                    cx.stop_propagation();
                    return;
                }
                // Typing jumps to the item starting with the letters.
                let Some(ch) = ev.keystroke.key_char.as_deref().filter(|ch| !ch.trim().is_empty()) else {
                    return;
                };
                let found = typing.update(cx, |s, _| s.type_to_select(ch, &key_items, lead));
                if let Some(i) = found {
                    key_cursor.update(cx, |s, _| s.anchor = i);
                    key_select(vec![i], i, window, cx);
                }
                cx.stop_propagation();
            })
            .child(rows)
    }
}

// --------------------------------------------------------------- check-list

/// One line of a [`CheckList`].
#[derive(Clone, Debug, PartialEq)]
pub struct CheckItem {
    pub label: SharedString,
    pub checked: bool,
}

impl CheckItem {
    pub fn new(label: impl Into<SharedString>, checked: bool) -> CheckItem {
        CheckItem {
            label: label.into(),
            checked,
        }
    }
}

/// A list of check boxes headed by "(Select All)", with an optional search
/// box that narrows the list as the user types: the list of values of a
/// filter drop-down.
///
/// The view owns the items; the list reports what the user ticks. Up /
/// Down move through the lines, Space ticks the current one.
///
/// ```ignore
/// CheckList::new("values", self.items.clone())
///     .search(&self.search)                 // made with widgets::input
///     .height(200.0)
///     .on_toggle(cx.listener(|this, (ix, on): &(usize, bool), _, cx| {
///         this.items[*ix].checked = *on;
///         cx.notify();
///     }))
///     .on_toggle_all(cx.listener(|this, (shown, on): &(Vec<usize>, bool), _, cx| {
///         for ix in shown { this.items[*ix].checked = *on; }
///         cx.notify();
///     }))
/// ```
#[derive(IntoElement)]
pub struct CheckList {
    id: ElementId,
    items: Vec<CheckItem>,
    search: Option<Entity<TextInput>>,
    width: Option<f32>,
    height: f32,
    all_label: SharedString,
    on_toggle: Option<Handler<(usize, bool)>>,
    on_toggle_all: Option<Handler<(Vec<usize>, bool)>>,
}

impl CheckList {
    pub fn new(id: impl Into<ElementId>, items: Vec<CheckItem>) -> CheckList {
        CheckList {
            id: id.into(),
            items,
            search: None,
            width: None,
            height: ROW_H * 9.0,
            all_label: crate::i18n::t("(Select All)", "(Sélectionner tout)").into(),
            on_toggle: None,
            on_toggle_all: None,
        }
    }

    /// Show a search box above the list and keep only the items containing
    /// what is typed in it.
    pub fn search(mut self, input: &Entity<TextInput>) -> Self {
        self.search = Some(input.clone());
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Height of the list itself (the search box comes on top of it).
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// An item was ticked or unticked: its index in the items given, and
    /// its new state.
    pub fn on_toggle(mut self, f: impl Fn(&(usize, bool), &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    /// "(Select All)" was ticked or unticked: the indices of the items
    /// currently listed (all of them without a search), and the new state.
    pub fn on_toggle_all(mut self, f: impl Fn(&(Vec<usize>, bool), &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle_all = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for CheckList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollState::new());
        let cursor = window.use_keyed_state(self.id.clone(), cx, |_, _| ListCursor::new());
        let height = self.height;
        let needle = self
            .search
            .as_ref()
            .map(|s| s.read(cx).text().trim().to_lowercase())
            .unwrap_or_default();
        let searching = !needle.is_empty();
        let items = Rc::new(self.items);
        // Indices of the items listed.
        let shown: Rc<Vec<usize>> = Rc::new(
            items
                .iter()
                .enumerate()
                .filter(|(_, item)| !searching || item.label.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect(),
        );
        let ticked = shown.iter().filter(|i| items[**i].checked).count();
        let all = if ticked == 0 {
            Tri::Off
        } else if ticked == shown.len() {
            Tri::On
        } else {
            Tri::Mixed
        };
        let all_label: SharedString = if searching {
            crate::i18n::t("(Select All Search Results)", "(Sélectionner tous les résultats de la recherche)").into()
        } else {
            self.all_label
        };
        // Row 0 is "(Select All)", absent when the search finds nothing.
        let count = if shown.is_empty() { 0 } else { shown.len() + 1 };
        let at = cursor.read(cx).cursor.min(count.saturating_sub(1));

        // Tick or untick the line at `row`.
        let toggle: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let (on_toggle, on_toggle_all) = (self.on_toggle.clone(), self.on_toggle_all.clone());
            let (items, shown, cursor) = (items.clone(), shown.clone(), cursor.clone());
            Rc::new(move |row: usize, window: &mut Window, cx: &mut App| {
                cursor.update(cx, |s, cx| {
                    s.cursor = row;
                    cx.notify();
                });
                if row == 0 {
                    if let Some(f) = &on_toggle_all {
                        f(&((*shown).clone(), all != Tri::On), window, cx);
                    }
                } else if let (Some(f), Some(ix)) = (&on_toggle, shown.get(row - 1)) {
                    f(&(*ix, !items[*ix].checked), window, cx);
                }
            })
        };

        let rows = virtual_rows(&scroll, count, ROW_H, height, cx, |row| {
            let toggle = toggle.clone();
            let (state, text) = if row == 0 {
                (all, all_label.clone())
            } else {
                let item = &items[shown[row - 1]];
                (Tri::from(item.checked), item.label.clone())
            };
            div()
                .id(("row", row))
                .size_full()
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when(focused && row == at, |d| d.bg(c(SELECTED_BG)))
                .when(!(focused && row == at), |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .child(check_mark(state, false))
                .child(div().truncate().child(text))
                .on_click(move |_, window, cx| toggle(row, window, cx))
                .into_any_element()
        });

        let key_cursor = cursor.clone();
        let key_scroll = scroll.clone();
        let key_toggle = toggle.clone();
        let page = (height / ROW_H) as usize;
        let list = list_frame(focused, None)
            .w_full()
            .id(self.id)
            .track_focus(&focus)
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if count == 0 || !plain(ev) {
                    return;
                }
                let key = ev.keystroke.key.as_str();
                if key == "space" {
                    key_toggle(at, window, cx);
                    cx.stop_propagation();
                } else if let Some(next) = navigate(key, Some(at), count, page) {
                    key_cursor.update(cx, |s, cx| {
                        s.cursor = next;
                        cx.notify();
                    });
                    key_scroll.update(cx, |s, _| s.reveal(next, next, ROW_H, height));
                    cx.stop_propagation();
                }
            })
            .child(rows);

        div()
            .when_some(self.width, |d, w| d.w(px(w)).flex_none())
            .when(self.width.is_none(), |d| d.flex_1())
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(self.search.as_ref().map(|search| {
                div()
                    .h(px(CONTROL_H))
                    .flex()
                    .child(TextField::new(search).trailing(div().pr(px(4.)).child(icon("search", 12.0, theme::TEXT_MUTED))))
            }))
            .child(list)
    }
}

// --------------------------------------------------------------------- tree

/// A node of a [`Tree`]. A node with children is a branch: its check box
/// reflects its leaves (checked, unchecked, or mixed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeNode {
    pub label: SharedString,
    /// For a leaf; ignored on a branch.
    pub checked: bool,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn leaf(label: impl Into<SharedString>, checked: bool) -> TreeNode {
        TreeNode {
            label: label.into(),
            checked,
            expanded: false,
            children: Vec::new(),
        }
    }

    pub fn branch(label: impl Into<SharedString>, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            label: label.into(),
            checked: false,
            expanded: false,
            children,
        }
    }

    /// The state of the node's check box.
    pub fn state(&self) -> Tri {
        if self.children.is_empty() {
            return Tri::from(self.checked);
        }
        let mut states = self.children.iter().map(|child| child.state());
        let first = states.next().unwrap_or_default();
        if states.all(|s| s == first) { first } else { Tri::Mixed }
    }

    /// Tick or untick the node and everything under it.
    pub fn set_checked(&mut self, on: bool) {
        self.checked = on;
        for child in &mut self.children {
            child.set_checked(on);
        }
    }

    /// The node at a path of child indices from the roots.
    pub fn at_mut<'a>(roots: &'a mut [TreeNode], path: &[usize]) -> Option<&'a mut TreeNode> {
        let (first, rest) = path.split_first()?;
        let node = roots.get_mut(*first)?;
        if rest.is_empty() {
            Some(node)
        } else {
            TreeNode::at_mut(&mut node.children, rest)
        }
    }

    /// What a click on a check box does: a checked node is unticked, any
    /// other one ticked, with everything under it.
    pub fn toggle(roots: &mut [TreeNode], path: &[usize]) {
        if let Some(node) = TreeNode::at_mut(roots, path) {
            let on = node.state() != Tri::On;
            node.set_checked(on);
        }
    }

    pub fn toggle_expanded(roots: &mut [TreeNode], path: &[usize]) {
        if let Some(node) = TreeNode::at_mut(roots, path) {
            node.expanded = !node.expanded;
        }
    }
}

/// One visible line of a tree.
struct TreeRow {
    path: Vec<usize>,
    label: SharedString,
    state: Tri,
    /// `None` for a leaf.
    expanded: Option<bool>,
}

fn flatten(nodes: &[TreeNode], prefix: &mut Vec<usize>, out: &mut Vec<TreeRow>) {
    for (i, node) in nodes.iter().enumerate() {
        prefix.push(i);
        out.push(TreeRow {
            path: prefix.clone(),
            label: node.label.clone(),
            state: node.state(),
            expanded: (!node.children.is_empty()).then_some(node.expanded),
        });
        if node.expanded {
            flatten(&node.children, prefix, out);
        }
        prefix.pop();
    }
}

/// A tree whose branches expand and collapse, with optional tri-state
/// check boxes (the years / months / days of a date filter, the fields of
/// a pivot table).
///
/// The view owns the nodes and changes them when the tree reports a click
/// (`TreeNode::toggle` and `TreeNode::toggle_expanded` do the usual thing).
/// Up / Down move through the lines, Right / Left expand and collapse,
/// Space ticks.
///
/// ```ignore
/// Tree::new("dates", &self.years)
///     .height(180.0)
///     .on_toggle(cx.listener(|this, path: &Vec<usize>, _, cx| { TreeNode::toggle(&mut this.years, path); cx.notify() }))
///     .on_expand(cx.listener(|this, path: &Vec<usize>, _, cx| { TreeNode::toggle_expanded(&mut this.years, path); cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct Tree {
    id: ElementId,
    rows: Vec<TreeRow>,
    checks: bool,
    width: Option<f32>,
    height: f32,
    on_toggle: Option<Handler<Vec<usize>>>,
    on_expand: Option<Handler<Vec<usize>>>,
    on_select: Option<Handler<Vec<usize>>>,
}

impl Tree {
    pub fn new(id: impl Into<ElementId>, roots: &[TreeNode]) -> Tree {
        let mut rows = Vec::new();
        flatten(roots, &mut Vec::new(), &mut rows);
        Tree {
            id: id.into(),
            rows,
            checks: true,
            width: None,
            height: ROW_H * 9.0,
            on_toggle: None,
            on_expand: None,
            on_select: None,
        }
    }

    /// A tree without check boxes.
    pub fn plain(mut self) -> Self {
        self.checks = false;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// The check box of the node at this path was clicked.
    pub fn on_toggle(mut self, f: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    /// The branch at this path is to be expanded or collapsed.
    pub fn on_expand(mut self, f: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> Self {
        self.on_expand = Some(Rc::new(f));
        self
    }

    /// The line of the node at this path was clicked or reached with the
    /// arrows.
    pub fn on_select(mut self, f: impl Fn(&Vec<usize>, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Tree {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollState::new());
        let cursor = window.use_keyed_state(self.id.clone(), cx, |_, _| ListCursor::new());
        let height = self.height;
        let rows = Rc::new(self.rows);
        let count = rows.len();
        let at = cursor.read(cx).cursor.min(count.saturating_sub(1));
        let checks = self.checks;
        let (on_toggle, on_expand, on_select) = (self.on_toggle, self.on_expand, self.on_select);

        let go: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let (cursor, scroll, rows, on_select) = (cursor.clone(), scroll.clone(), rows.clone(), on_select.clone());
            Rc::new(move |row: usize, window: &mut Window, cx: &mut App| {
                cursor.update(cx, |s, cx| {
                    s.cursor = row;
                    cx.notify();
                });
                scroll.update(cx, |s, _| s.reveal(row, row, ROW_H, height));
                if let (Some(f), Some(line)) = (&on_select, rows.get(row)) {
                    f(&line.path, window, cx);
                }
            })
        };

        let list = virtual_rows(&scroll, count, ROW_H, height, cx, |i| {
            let line = &rows[i];
            let depth = line.path.len() - 1;
            let expander = match line.expanded {
                Some(open) => {
                    let (on_expand, path) = (on_expand.clone(), line.path.clone());
                    div()
                        .id(("expand", i))
                        .size(px(14.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(2.))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 9.0, theme::TEXT_MUTED))
                        .on_click(move |_, window, cx| {
                            if let Some(f) = &on_expand {
                                f(&path, window, cx);
                            }
                            cx.stop_propagation();
                        })
                        .into_any_element()
                }
                None => div().size(px(14.)).flex_none().into_any_element(),
            };
            let check = checks.then(|| {
                let (on_toggle, path) = (on_toggle.clone(), line.path.clone());
                div()
                    .id(("check", i))
                    .child(check_mark(line.state, false))
                    .on_click(move |_, window, cx| {
                        if let Some(f) = &on_toggle {
                            f(&path, window, cx);
                        }
                    })
            });
            let go = go.clone();
            div()
                .id(("row", i))
                .size_full()
                .pl(px(4.0 + depth as f32 * 16.0))
                .pr(px(6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .when(focused && i == at, |d| d.bg(c(SELECTED_BG)))
                .when(!(focused && i == at), |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .child(expander)
                .children(check)
                .child(div().truncate().child(line.label.clone()))
                .on_click(move |_, window, cx| go(i, window, cx))
                .into_any_element()
        });

        let key_rows = rows.clone();
        let page = (height / ROW_H) as usize;
        list_frame(focused, self.width)
            .id(self.id)
            .track_focus(&focus)
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if count == 0 || !plain(ev) {
                    return;
                }
                let line = &key_rows[at];
                let key = ev.keystroke.key.as_str();
                match (key, line.expanded) {
                    ("space", _) if checks => {
                        if let Some(f) = &on_toggle {
                            f(&line.path, window, cx);
                        }
                    }
                    ("right", Some(false)) | ("left", Some(true)) => {
                        if let Some(f) = &on_expand {
                            f(&line.path, window, cx);
                        }
                    }
                    // Left on a child goes to its parent.
                    ("left", _) if line.path.len() > 1 => {
                        let parent = &line.path[..line.path.len() - 1];
                        if let Some(row) = key_rows.iter().position(|r| r.path == parent) {
                            go(row, window, cx);
                        }
                    }
                    _ => match navigate(key, Some(at), count, page) {
                        Some(next) => go(next, window, cx),
                        None => return,
                    },
                }
                cx.stop_propagation();
            })
            .child(list)
    }
}
