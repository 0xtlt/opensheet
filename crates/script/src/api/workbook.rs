use super::{ApiClass, Ret, class};
pub(super) const WORKBOOK: ApiClass = class!("Workbook", "The workbook the script runs on.", {
    getActiveWorksheet "(): Worksheet" "The sheet in front." => |h, _| Ok(Ret::Object(h.worksheet(h.active_sheet())));
});
pub(super) const APPLICATION: ApiClass = class!("Application", "", {});
pub(super) const NAMED_ITEM: ApiClass = class!("NamedItem", "", {});
