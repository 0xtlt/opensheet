//! The enums of the `ExcelScript` namespace, with Excel's names and
//! values. They list every member Excel has: a member OpenSheet cannot
//! honour is refused by the method it is given to, with its name.

use super::ApiEnum;

macro_rules! members {
    ($( $name:ident $value:literal ),* $(,)?) => { &[ $( (stringify!($name), $value) ),* ] };
}

pub(crate) static ENUMS: &[ApiEnum] = &[
    ApiEnum {
        name: "AutoFillType",
        members: members!(fillDefault "FillDefault", fillCopy "FillCopy", fillSeries "FillSeries", fillFormats "FillFormats", fillValues "FillValues", fillDays "FillDays", fillWeekdays "FillWeekdays", fillMonths "FillMonths", fillYears "FillYears", linearTrend "LinearTrend", growthTrend "GrowthTrend", flashFill "FlashFill"),
    },
    ApiEnum {
        name: "BorderIndex",
        members: members!(edgeTop "EdgeTop", edgeBottom "EdgeBottom", edgeLeft "EdgeLeft", edgeRight "EdgeRight", insideVertical "InsideVertical", insideHorizontal "InsideHorizontal", diagonalDown "DiagonalDown", diagonalUp "DiagonalUp"),
    },
    ApiEnum {
        name: "BorderLineStyle",
        members: members!(none "None", continuous "Continuous", dash "Dash", dashDot "DashDot", dashDotDot "DashDotDot", dot "Dot", double "Double", slantDashDot "SlantDashDot"),
    },
    ApiEnum {
        name: "BorderWeight",
        members: members!(hairline "Hairline", thin "Thin", medium "Medium", thick "Thick"),
    },
    ApiEnum {
        name: "CalculationMode",
        members: members!(automatic "Automatic", automaticExceptTables "AutomaticExceptTables", manual "Manual"),
    },
    ApiEnum {
        name: "CalculationType",
        members: members!(recalculate "Recalculate", full "Full", fullRebuild "FullRebuild"),
    },
    ApiEnum {
        name: "ClearApplyTo",
        members: members!(all "All", formats "Formats", contents "Contents", hyperlinks "Hyperlinks", removeHyperlinks "RemoveHyperlinks"),
    },
    ApiEnum {
        name: "DeleteShiftDirection",
        members: members!(up "Up", left "Left"),
    },
    ApiEnum {
        name: "DynamicFilterCriteria",
        members: members!(unknown "Unknown", aboveAverage "AboveAverage", allDatesInPeriodApril "AllDatesInPeriodApril", allDatesInPeriodAugust "AllDatesInPeriodAugust", allDatesInPeriodDecember "AllDatesInPeriodDecember", allDatesInPeriodFebruray "AllDatesInPeriodFebruray", allDatesInPeriodJanuary "AllDatesInPeriodJanuary", allDatesInPeriodJuly "AllDatesInPeriodJuly", allDatesInPeriodJune "AllDatesInPeriodJune", allDatesInPeriodMarch "AllDatesInPeriodMarch", allDatesInPeriodMay "AllDatesInPeriodMay", allDatesInPeriodNovember "AllDatesInPeriodNovember", allDatesInPeriodOctober "AllDatesInPeriodOctober", allDatesInPeriodQuarter1 "AllDatesInPeriodQuarter1", allDatesInPeriodQuarter2 "AllDatesInPeriodQuarter2", allDatesInPeriodQuarter3 "AllDatesInPeriodQuarter3", allDatesInPeriodQuarter4 "AllDatesInPeriodQuarter4", allDatesInPeriodSeptember "AllDatesInPeriodSeptember", belowAverage "BelowAverage", lastMonth "LastMonth", lastQuarter "LastQuarter", lastWeek "LastWeek", lastYear "LastYear", nextMonth "NextMonth", nextQuarter "NextQuarter", nextWeek "NextWeek", nextYear "NextYear", thisMonth "ThisMonth", thisQuarter "ThisQuarter", thisWeek "ThisWeek", thisYear "ThisYear", today "Today", tomorrow "Tomorrow", yearToDate "YearToDate", yesterday "Yesterday"),
    },
    ApiEnum {
        name: "FilterOn",
        members: members!(bottomItems "BottomItems", bottomPercent "BottomPercent", cellColor "CellColor", dynamic "Dynamic", fontColor "FontColor", values "Values", topItems "TopItems", topPercent "TopPercent", icon "Icon", custom "Custom"),
    },
    ApiEnum {
        name: "FilterOperator",
        members: members!(and "And", or "Or"),
    },
    ApiEnum {
        name: "HorizontalAlignment",
        members: members!(general "General", left "Left", center "Center", right "Right", fill "Fill", justify "Justify", centerAcrossSelection "CenterAcrossSelection", distributed "Distributed"),
    },
    ApiEnum {
        name: "InsertShiftDirection",
        members: members!(down "Down", right "Right"),
    },
    ApiEnum {
        name: "KeyboardDirection",
        members: members!(left "Left", right "Right", up "Up", down "Down"),
    },
    ApiEnum {
        name: "NamedItemScope",
        members: members!(worksheet "Worksheet", workbook "Workbook"),
    },
    ApiEnum {
        name: "NamedItemType",
        members: members!(string "String", integer "Integer", double "Double", boolean "Boolean", range "Range", error "Error", array "Array"),
    },
    ApiEnum {
        name: "RangeCopyType",
        members: members!(all "All", formulas "Formulas", values "Values", formats "Formats", link "Link"),
    },
    ApiEnum {
        name: "RangeUnderlineStyle",
        members: members!(none "None", single "Single", double "Double", singleAccountant "SingleAccountant", doubleAccountant "DoubleAccountant"),
    },
    ApiEnum {
        name: "RangeValueType",
        members: members!(unknown "Unknown", empty "Empty", string "String", integer "Integer", double "Double", boolean "Boolean", error "Error", richValue "RichValue"),
    },
    ApiEnum {
        name: "SearchDirection",
        members: members!(forward "Forward", backwards "Backwards"),
    },
    ApiEnum {
        name: "SheetVisibility",
        members: members!(visible "Visible", hidden "Hidden", veryHidden "VeryHidden"),
    },
    ApiEnum {
        name: "SortDataOption",
        members: members!(normal "Normal", textAsNumber "TextAsNumber"),
    },
    ApiEnum {
        name: "SortMethod",
        members: members!(pinYin "PinYin", strokeCount "StrokeCount"),
    },
    ApiEnum {
        name: "SortOn",
        members: members!(value "Value", cellColor "CellColor", fontColor "FontColor", icon "Icon"),
    },
    ApiEnum {
        name: "SortOrientation",
        members: members!(rows "Rows", columns "Columns"),
    },
    ApiEnum {
        name: "VerticalAlignment",
        members: members!(top "Top", center "Center", bottom "Bottom", justify "Justify", distributed "Distributed"),
    },
    ApiEnum {
        name: "WorksheetPositionType",
        members: members!(none "None", before "Before", after "After", beginning "Beginning", end "End"),
    },
];
