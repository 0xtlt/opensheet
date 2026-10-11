use std::fmt;
use std::sync::Arc;

/// Spreadsheet error values (`#DIV/0!`, `#N/A`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    Spill,
    Calc,
}

impl ErrorKind {
    pub const ALL: [ErrorKind; 9] = [
        ErrorKind::Null,
        ErrorKind::Div0,
        ErrorKind::Value,
        ErrorKind::Ref,
        ErrorKind::Name,
        ErrorKind::Num,
        ErrorKind::NA,
        ErrorKind::Spill,
        ErrorKind::Calc,
    ];

    /// Canonical (English) spelling, as stored in files.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Null => "#NULL!",
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
            ErrorKind::Spill => "#SPILL!",
            ErrorKind::Calc => "#CALC!",
        }
    }

    pub fn from_str(s: &str) -> Option<ErrorKind> {
        let up = s.to_ascii_uppercase();
        ErrorKind::ALL.into_iter().find(|e| e.as_str() == up)
    }

    /// The number returned by `ERROR.TYPE`.
    pub fn type_number(self) -> f64 {
        match self {
            ErrorKind::Null => 1.0,
            ErrorKind::Div0 => 2.0,
            ErrorKind::Value => 3.0,
            ErrorKind::Ref => 4.0,
            ErrorKind::Name => 5.0,
            ErrorKind::Num => 6.0,
            ErrorKind::NA => 7.0,
            ErrorKind::Spill => 9.0,
            ErrorKind::Calc => 14.0,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A rectangular block of scalar values, stored row-major.
/// Elements are never themselves `Value::Array`.
#[derive(Clone, Debug, PartialEq)]
pub struct Array {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Value>,
}

static EMPTY: Value = Value::Empty;

impl Array {
    pub fn new(rows: usize, cols: usize) -> Array {
        Array {
            rows,
            cols,
            data: vec![Value::Empty; rows * cols],
        }
    }

    pub fn from_rows(rows: Vec<Vec<Value>>) -> Array {
        let r = rows.len();
        let c = rows.iter().map(|x| x.len()).max().unwrap_or(0);
        let mut data = Vec::with_capacity(r * c);
        for row in rows {
            let len = row.len();
            data.extend(row);
            data.extend(std::iter::repeat_n(Value::Empty, c - len));
        }
        Array {
            rows: r,
            cols: c,
            data,
        }
    }

    /// A single-column array.
    pub fn column(values: Vec<Value>) -> Array {
        Array {
            rows: values.len(),
            cols: 1,
            data: values,
        }
    }

    /// A single-row array.
    pub fn row(values: Vec<Value>) -> Array {
        Array {
            rows: 1,
            cols: values.len(),
            data: values,
        }
    }

    /// Element at (row, col); `Empty` when out of bounds.
    pub fn get(&self, row: usize, col: usize) -> &Value {
        if row < self.rows && col < self.cols {
            &self.data[row * self.cols + col]
        } else {
            &EMPTY
        }
    }

    pub fn set(&mut self, row: usize, col: usize, v: Value) {
        if row < self.rows && col < self.cols {
            self.data[row * self.cols + col] = v;
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.data.iter()
    }

    pub fn row_slice(&self, row: usize) -> &[Value] {
        &self.data[row * self.cols..(row + 1) * self.cols]
    }

    /// Element with broadcasting: a dimension of size 1 repeats; otherwise
    /// out-of-range yields `#N/A`.
    pub fn get_broadcast(&self, row: usize, col: usize) -> Value {
        let r = if self.rows == 1 { 0 } else { row };
        let c = if self.cols == 1 { 0 } else { col };
        if r < self.rows && c < self.cols {
            self.data[r * self.cols + c].clone()
        } else {
            Value::Error(ErrorKind::NA)
        }
    }
}

/// A computed spreadsheet value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Empty,
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(ErrorKind),
    Array(Arc<Array>),
}

impl Value {
    pub fn text(s: impl AsRef<str>) -> Value {
        Value::Text(Arc::from(s.as_ref()))
    }

    pub fn array(a: Array) -> Value {
        Value::Array(Arc::new(a))
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Value::Text(_))
    }

    pub fn as_error(&self) -> Option<ErrorKind> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// Collapse an array to its top-left element; scalars are returned as is.
    pub fn into_scalar(self) -> Value {
        match self {
            Value::Array(a) => a.data.first().cloned().unwrap_or(Value::Empty),
            v => v,
        }
    }

    /// Dimensions as (rows, cols); scalars are 1x1.
    pub fn dims(&self) -> (usize, usize) {
        match self {
            Value::Array(a) => (a.rows, a.cols),
            _ => (1, 1),
        }
    }

    /// Element with broadcasting (scalars repeat everywhere).
    pub fn get_broadcast(&self, row: usize, col: usize) -> Value {
        match self {
            Value::Array(a) => a.get_broadcast(row, col),
            v => v.clone(),
        }
    }

    /// View any value as an array (scalars become 1x1).
    pub fn to_array(&self) -> Arc<Array> {
        match self {
            Value::Array(a) => a.clone(),
            v => Arc::new(Array {
                rows: 1,
                cols: 1,
                data: vec![v.clone()],
            }),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::text(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Text(Arc::from(s))
    }
}

impl From<ErrorKind> for Value {
    fn from(e: ErrorKind) -> Value {
        Value::Error(e)
    }
}

/// Result type used by function implementations: `Err` becomes an error value.
pub type R<T> = Result<T, ErrorKind>;
