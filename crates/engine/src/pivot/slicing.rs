//! What slicers ask of a PivotTable beyond its report.

use super::{FilterKind, PivotTable};
use crate::value::Value;

impl PivotTable {
    /// For every item of `field`, whether the other filters of the
    /// PivotTable (the items unchecked in its other fields, the selection
    /// of its filter fields, its date filters) leave a record with it: a
    /// slicer greys the others out.
    pub fn items_with_data(&self, field: usize) -> Vec<bool> {
        let cache = &self.cache;
        let mut keep = vec![true; cache.rows];
        let mut drop_hidden = |f: usize, visible: &dyn Fn(u32, &Value) -> bool| {
            let items = cache.items(f);
            let shown: Vec<bool> = items.iter().enumerate().map(|(x, v)| visible(x as u32, v)).collect();
            if shown.iter().all(|v| *v) {
                return;
            }
            for (k, x) in keep.iter_mut().zip(cache.item_column(f).iter()) {
                *k &= shown.get(*x as usize).copied().unwrap_or(false);
            }
        };
        for f in (0..self.fields.len()).filter(|f| *f != field) {
            let page = self.page_fields.iter().find(|p| p.field == f);
            let applies = page.is_some() || self.axis_of(f).is_some() || self.sliced.contains(&f);
            match page.and_then(|p| p.item) {
                Some(only) => drop_hidden(f, &|x, _| x == only),
                None if applies => {
                    let hidden: Vec<u32> = self.fields[f].items.iter().filter(|it| it.hidden).map(|it| it.x).collect();
                    if !hidden.is_empty() {
                        drop_hidden(f, &|x, _| !hidden.contains(&x));
                    }
                }
                None => {}
            }
            for filter in self.filters.iter().filter(|x| x.field == f) {
                if let FilterKind::Date { start, end } = filter.kind {
                    drop_hidden(f, &|_, v| matches!(v, Value::Number(n) if n.floor() >= start && n.floor() <= end));
                }
            }
        }
        let mut out = vec![false; cache.items(field).len()];
        for (k, x) in keep.iter().zip(cache.item_column(field).iter()) {
            if *k && let Some(seen) = out.get_mut(*x as usize) {
                *seen = true;
            }
        }
        out
    }
}
