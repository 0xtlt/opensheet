//! The enums of the `ExcelScript` namespace, with Excel's names and
//! values. They list every member Excel has: a member OpenSheet cannot
//! honour is refused by the method it is given to, with its name.
//! `ChartType` and `GeometricShapeType` alone list only the charts and
//! the shapes OpenSheet draws: reading another of their members fails,
//! with its name.

use super::ApiEnum;

macro_rules! members {
    ($( $name:ident $value:literal ),* $(,)?) => { &[ $( (stringify!($name), $value) ),* ] };
}

pub(crate) static ENUMS: &[ApiEnum] = &[
    ApiEnum {
        name: "AggregationFunction",
        members: members!(unknown "Unknown", automatic "Automatic", sum "Sum", count "Count", average "Average", max "Max", min "Min", product "Product", countNumbers "CountNumbers", standardDeviation "StandardDeviation", standardDeviationP "StandardDeviationP", variance "Variance", varianceP "VarianceP"),
    },
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
        name: "ChartLegendPosition",
        members: members!(invalid "Invalid", top "Top", bottom "Bottom", left "Left", right "Right", corner "Corner", custom "Custom"),
    },
    ApiEnum {
        name: "ChartSeriesBy",
        members: members!(auto "Auto", columns "Columns", rows "Rows"),
    },
    ApiEnum {
        name: "ChartType",
        members: members!(columnClustered "ColumnClustered", columnStacked "ColumnStacked", columnStacked100 "ColumnStacked100", _3DColumnClustered "3DColumnClustered", _3DColumnStacked "3DColumnStacked", _3DColumnStacked100 "3DColumnStacked100", line "Line", lineStacked "LineStacked", lineStacked100 "LineStacked100", lineMarkers "LineMarkers", lineMarkersStacked "LineMarkersStacked", lineMarkersStacked100 "LineMarkersStacked100", _3DLine "3DLine", pie "Pie", _3DPie "3DPie", doughnut "Doughnut", barClustered "BarClustered", barStacked "BarStacked", barStacked100 "BarStacked100", _3DBarClustered "3DBarClustered", _3DBarStacked "3DBarStacked", _3DBarStacked100 "3DBarStacked100", area "Area", areaStacked "AreaStacked", areaStacked100 "AreaStacked100", _3DArea "3DArea", _3DAreaStacked "3DAreaStacked", _3DAreaStacked100 "3DAreaStacked100", xyscatter "XYScatter", xyscatterSmooth "XYScatterSmooth", xyscatterSmoothNoMarkers "XYScatterSmoothNoMarkers", xyscatterLines "XYScatterLines", xyscatterLinesNoMarkers "XYScatterLinesNoMarkers", bubble "Bubble", radar "Radar", radarMarkers "RadarMarkers", radarFilled "RadarFilled"),
    },
    ApiEnum {
        name: "ClearApplyTo",
        members: members!(all "All", formats "Formats", contents "Contents", hyperlinks "Hyperlinks", removeHyperlinks "RemoveHyperlinks"),
    },
    ApiEnum {
        name: "ConditionalCellValueOperator",
        members: members!(invalid "Invalid", between "Between", notBetween "NotBetween", equalTo "EqualTo", notEqualTo "NotEqualTo", greaterThan "GreaterThan", lessThan "LessThan", greaterThanOrEqual "GreaterThanOrEqual", lessThanOrEqual "LessThanOrEqual"),
    },
    ApiEnum {
        name: "ConditionalDataBarAxisFormat",
        members: members!(automatic "Automatic", none "None", cellMidPoint "CellMidPoint"),
    },
    ApiEnum {
        name: "ConditionalDataBarDirection",
        members: members!(context "Context", leftToRight "LeftToRight", rightToLeft "RightToLeft"),
    },
    ApiEnum {
        name: "ConditionalFormatColorCriterionType",
        members: members!(invalid "Invalid", lowestValue "LowestValue", highestValue "HighestValue", number "Number", percent "Percent", formula "Formula", percentile "Percentile"),
    },
    ApiEnum {
        name: "ConditionalFormatIconRuleType",
        members: members!(invalid "Invalid", number "Number", percent "Percent", formula "Formula", percentile "Percentile"),
    },
    ApiEnum {
        name: "ConditionalFormatPresetCriterion",
        members: members!(invalid "Invalid", blanks "Blanks", nonBlanks "NonBlanks", errors "Errors", nonErrors "NonErrors", yesterday "Yesterday", today "Today", tomorrow "Tomorrow", lastSevenDays "LastSevenDays", lastWeek "LastWeek", thisWeek "ThisWeek", nextWeek "NextWeek", lastMonth "LastMonth", thisMonth "ThisMonth", nextMonth "NextMonth", aboveAverage "AboveAverage", belowAverage "BelowAverage", equalOrAboveAverage "EqualOrAboveAverage", equalOrBelowAverage "EqualOrBelowAverage", oneStdDevAboveAverage "OneStdDevAboveAverage", oneStdDevBelowAverage "OneStdDevBelowAverage", twoStdDevAboveAverage "TwoStdDevAboveAverage", twoStdDevBelowAverage "TwoStdDevBelowAverage", threeStdDevAboveAverage "ThreeStdDevAboveAverage", threeStdDevBelowAverage "ThreeStdDevBelowAverage", uniqueValues "UniqueValues", duplicateValues "DuplicateValues"),
    },
    ApiEnum {
        name: "ConditionalFormatRuleType",
        members: members!(invalid "Invalid", automatic "Automatic", lowestValue "LowestValue", highestValue "HighestValue", number "Number", percent "Percent", formula "Formula", percentile "Percentile"),
    },
    ApiEnum {
        name: "ConditionalFormatType",
        members: members!(custom "Custom", dataBar "DataBar", colorScale "ColorScale", iconSet "IconSet", topBottom "TopBottom", presetCriteria "PresetCriteria", containsText "ContainsText", cellValue "CellValue"),
    },
    ApiEnum {
        name: "ConditionalIconCriterionOperator",
        members: members!(invalid "Invalid", greaterThan "GreaterThan", greaterThanOrEqual "GreaterThanOrEqual"),
    },
    ApiEnum {
        name: "ConditionalRangeBorderIndex",
        members: members!(edgeTop "EdgeTop", edgeBottom "EdgeBottom", edgeLeft "EdgeLeft", edgeRight "EdgeRight"),
    },
    ApiEnum {
        name: "ConditionalRangeBorderLineStyle",
        members: members!(none "None", continuous "Continuous", dash "Dash", dashDot "DashDot", dashDotDot "DashDotDot", dot "Dot"),
    },
    ApiEnum {
        name: "ConditionalRangeFontUnderlineStyle",
        members: members!(none "None", single "Single", double "Double"),
    },
    ApiEnum {
        name: "ConditionalTextOperator",
        members: members!(invalid "Invalid", contains "Contains", notContains "NotContains", beginsWith "BeginsWith", endsWith "EndsWith"),
    },
    ApiEnum {
        name: "ConditionalTopBottomCriterionType",
        members: members!(invalid "Invalid", topItems "TopItems", topPercent "TopPercent", bottomItems "BottomItems", bottomPercent "BottomPercent"),
    },
    ApiEnum {
        name: "ConnectorType",
        members: members!(straight "Straight", elbow "Elbow", curve "Curve"),
    },
    ApiEnum {
        name: "ContentType",
        members: members!(plain "Plain", mention "Mention"),
    },
    ApiEnum {
        name: "DataValidationAlertStyle",
        members: members!(stop "Stop", warning "Warning", information "Information"),
    },
    ApiEnum {
        name: "DataValidationOperator",
        members: members!(between "Between", notBetween "NotBetween", equalTo "EqualTo", notEqualTo "NotEqualTo", greaterThan "GreaterThan", lessThan "LessThan", greaterThanOrEqualTo "GreaterThanOrEqualTo", lessThanOrEqualTo "LessThanOrEqualTo"),
    },
    ApiEnum {
        name: "DataValidationType",
        members: members!(none "None", wholeNumber "WholeNumber", decimal "Decimal", list "List", date "Date", time "Time", textLength "TextLength", custom "Custom", inconsistent "Inconsistent", mixedCriteria "MixedCriteria"),
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
        name: "GeometricShapeType",
        members: members!(rectangle "Rectangle", roundRectangle "RoundRectangle", ellipse "Ellipse", triangle "Triangle", rightTriangle "RightTriangle", diamond "Diamond", pentagon "Pentagon", hexagon "Hexagon", rightArrow "RightArrow", leftArrow "LeftArrow", upArrow "UpArrow", downArrow "DownArrow", star5 "Star5", wedgeRectCallout "WedgeRectCallout", chevron "Chevron", trapezoid "Trapezoid"),
    },
    ApiEnum {
        name: "HorizontalAlignment",
        members: members!(general "General", left "Left", center "Center", right "Right", fill "Fill", justify "Justify", centerAcrossSelection "CenterAcrossSelection", distributed "Distributed"),
    },
    ApiEnum {
        name: "IconSet",
        members: members!(invalid "Invalid", threeArrows "ThreeArrows", threeArrowsGray "ThreeArrowsGray", threeFlags "ThreeFlags", threeTrafficLights1 "ThreeTrafficLights1", threeTrafficLights2 "ThreeTrafficLights2", threeSigns "ThreeSigns", threeSymbols "ThreeSymbols", threeSymbols2 "ThreeSymbols2", fourArrows "FourArrows", fourArrowsGray "FourArrowsGray", fourRedToBlack "FourRedToBlack", fourRating "FourRating", fourTrafficLights "FourTrafficLights", fiveArrows "FiveArrows", fiveArrowsGray "FiveArrowsGray", fiveRating "FiveRating", fiveQuarters "FiveQuarters", threeStars "ThreeStars", threeTriangles "ThreeTriangles", fiveBoxes "FiveBoxes"),
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
        name: "LabelFilterCondition",
        members: members!(unknown "Unknown", beginsWith "BeginsWith", endsWith "EndsWith", contains "Contains", greaterThan "GreaterThan", greaterThanOrEqualTo "GreaterThanOrEqualTo", lessThan "LessThan", lessThanOrEqualTo "LessThanOrEqualTo", equals "Equals", between "Between"),
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
        name: "PictureFormat",
        members: members!(unknown "UNKNOWN", bmp "BMP", jpeg "JPEG", gif "GIF", png "PNG", svg "SVG"),
    },
    ApiEnum {
        name: "PivotFilterType",
        members: members!(unknown "Unknown", value "Value", manual "Manual", label "Label", date "Date"),
    },
    ApiEnum {
        name: "PivotLayoutType",
        members: members!(compact "Compact", tabular "Tabular", outline "Outline"),
    },
    ApiEnum {
        name: "Placement",
        members: members!(twoCell "TwoCell", oneCell "OneCell", absolute "Absolute"),
    },
    ApiEnum {
        name: "ProtectionSelectionMode",
        members: members!(normal "Normal", unlocked "Unlocked", none "None"),
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
        name: "ShowAsCalculation",
        members: members!(unknown "Unknown", none "None", percentOfGrandTotal "PercentOfGrandTotal", percentOfRowTotal "PercentOfRowTotal", percentOfColumnTotal "PercentOfColumnTotal", percentOfParentRowTotal "PercentOfParentRowTotal", percentOfParentColumnTotal "PercentOfParentColumnTotal", percentOfParentTotal "PercentOfParentTotal", percentOf "PercentOf", runningTotal "RunningTotal", percentRunningTotal "PercentRunningTotal", differenceFrom "DifferenceFrom", percentDifferenceFrom "PercentDifferenceFrom", rankAscending "RankAscending", rankDecending "RankDecending", index "Index"),
    },
    ApiEnum {
        name: "ShapeFillType",
        members: members!(noFill "NoFill", solid "Solid", gradient "Gradient", pattern "Pattern", pictureAndTexture "PictureAndTexture", mixed "Mixed"),
    },
    ApiEnum {
        name: "ShapeFontUnderlineStyle",
        members: members!(none "None", single "Single", double "Double", heavy "Heavy", dotted "Dotted", dottedHeavy "DottedHeavy", dash "Dash", dashHeavy "DashHeavy", dashLong "DashLong", dashLongHeavy "DashLongHeavy", dotDash "DotDash", dotDashHeavy "DotDashHeavy", dotDotDash "DotDotDash", dotDotDashHeavy "DotDotDashHeavy", wavy "Wavy", wavyHeavy "WavyHeavy", wavyDouble "WavyDouble"),
    },
    ApiEnum {
        name: "ShapeLineDashStyle",
        members: members!(dash "Dash", dashDot "DashDot", dashDotDot "DashDotDot", longDash "LongDash", longDashDot "LongDashDot", roundDot "RoundDot", solid "Solid", squareDot "SquareDot", longDashDotDot "LongDashDotDot", systemDash "SystemDash", systemDot "SystemDot", systemDashDot "SystemDashDot"),
    },
    ApiEnum {
        name: "ShapeTextHorizontalAlignment",
        members: members!(left "Left", center "Center", right "Right", justify "Justify", justifyLow "JustifyLow", distributed "Distributed", thaiDistributed "ThaiDistributed"),
    },
    ApiEnum {
        name: "ShapeTextVerticalAlignment",
        members: members!(top "Top", middle "Middle", bottom "Bottom", justified "Justified", distributed "Distributed"),
    },
    ApiEnum {
        name: "ShapeType",
        members: members!(unsupported "Unsupported", image "Image", geometricShape "GeometricShape", group "Group", line "Line"),
    },
    ApiEnum {
        name: "ShapeZOrder",
        members: members!(bringToFront "BringToFront", bringForward "BringForward", sendToBack "SendToBack", sendBackward "SendBackward"),
    },
    ApiEnum {
        name: "SheetVisibility",
        members: members!(visible "Visible", hidden "Hidden", veryHidden "VeryHidden"),
    },
    ApiEnum {
        name: "SortBy",
        members: members!(ascending "Ascending", descending "Descending"),
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
        name: "SpecialCellType",
        members: members!(conditionalFormats "ConditionalFormats", dataValidations "DataValidations", blanks "Blanks", constants "Constants", formulas "Formulas", sameConditionalFormat "SameConditionalFormat", sameDataValidation "SameDataValidation", visible "Visible"),
    },
    ApiEnum {
        name: "SpecialCellValueType",
        members: members!(all "All", errors "Errors", errorsLogical "ErrorsLogical", errorsNumbers "ErrorsNumbers", errorsText "ErrorsText", errorsLogicalNumber "ErrorsLogicalNumber", errorsLogicalText "ErrorsLogicalText", errorsNumberText "ErrorsNumberText", logical "Logical", logicalNumbers "LogicalNumbers", logicalText "LogicalText", logicalNumbersText "LogicalNumbersText", numbers "Numbers", numbersText "NumbersText", text "Text"),
    },
    ApiEnum {
        name: "SubtotalLocationType",
        members: members!(atTop "AtTop", atBottom "AtBottom", off "Off"),
    },
    ApiEnum {
        name: "TopBottomSelectionType",
        members: members!(items "Items", percent "Percent", sum "Sum"),
    },
    ApiEnum {
        name: "ValueFilterCondition",
        members: members!(unknown "Unknown", equals "Equals", greaterThan "GreaterThan", greaterThanOrEqualTo "GreaterThanOrEqualTo", lessThan "LessThan", lessThanOrEqualTo "LessThanOrEqualTo", between "Between", topN "TopN", bottomN "BottomN"),
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
