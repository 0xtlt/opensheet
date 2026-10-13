use super::{ApiClass, class};
pub(super) const WORKSHEET: ApiClass = class!("Worksheet", "", {
    getName "(): string" "The name of the sheet." => |h, c| { let s = h.sheet(c, c.this_int(0))?; Ok(h.wb.sheets[s].name.clone().into()) };
});
pub(super) const FREEZE_PANES: ApiClass = class!("WorksheetFreezePanes", "", {});
pub(super) const AUTO_FILTER: ApiClass = class!("AutoFilter", "", {});
