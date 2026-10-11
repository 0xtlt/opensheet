//! Engineering functions: number-base conversion, bit operations, unit
//! conversion.

use super::{Category, FunctionDef, Kind};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::value::{ErrorKind, R, Value};

/// Numeric argument; unlike the general coercion, booleans are rejected
/// (as the engineering functions do in Excel).
fn number(ctx: &EvalCtx, e: &Expr) -> R<f64> {
    match ctx.eval_scalar(e) {
        Value::Bool(_) => Err(ErrorKind::Value),
        v => ctx.to_number(&v),
    }
}

fn number_or(ctx: &EvalCtx, args: &[Expr], index: usize, default: f64) -> R<f64> {
    match args.get(index) {
        None | Some(Expr::Missing) => Ok(default),
        Some(e) => number(ctx, e),
    }
}

// ---- base conversion -----------------------------------------------------
//
// Binary, octal and hexadecimal numbers have at most 10 digits; a 10-digit
// number with its top bit set is negative (two's complement).

#[derive(Clone, Copy)]
enum Base {
    Bin,
    Oct,
    Hex,
}

impl Base {
    fn radix(self) -> u32 {
        match self {
            Base::Bin => 2,
            Base::Oct => 8,
            Base::Hex => 16,
        }
    }

    /// `radix^10`: the size of the 10-digit two's-complement space.
    fn modulus(self) -> i64 {
        (self.radix() as i64).pow(10)
    }

    fn min(self) -> i64 {
        -self.modulus() / 2
    }

    fn max(self) -> i64 {
        self.modulus() / 2 - 1
    }
}

/// Digits of a binary / octal / hexadecimal argument. Numbers are read by
/// their decimal digits (`BIN2DEC(101)`).
fn digits_arg(ctx: &EvalCtx, e: &Expr) -> R<String> {
    match ctx.eval_scalar(e) {
        Value::Text(s) => Ok(s.to_string()),
        Value::Empty => Ok(String::new()),
        Value::Number(n) => {
            if n < 0.0 || n.fract() != 0.0 || n >= 1e10 {
                return Err(ErrorKind::Num);
            }
            Ok(format!("{n:.0}"))
        }
        Value::Bool(_) => Err(ErrorKind::Value),
        Value::Error(err) => Err(err),
        Value::Array(_) => Err(ErrorKind::Value),
    }
}

/// Parse up to 10 digits in `base` into a signed number.
fn parse_base(digits: &str, base: Base) -> R<i64> {
    if digits.len() > 10 {
        return Err(ErrorKind::Num);
    }
    let mut n: i64 = 0;
    for ch in digits.chars() {
        let d = ch.to_digit(base.radix()).ok_or(ErrorKind::Num)?;
        n = n * base.radix() as i64 + d as i64;
    }
    if n > base.max() {
        n -= base.modulus();
    }
    Ok(n)
}

/// Format `n` in `base`, padded with zeros to `places` when given.
/// Negative numbers always use all 10 digits.
fn format_base(n: i64, base: Base, places: Option<f64>) -> R<Value> {
    if n < base.min() || n > base.max() {
        return Err(ErrorKind::Num);
    }
    let raw = if n < 0 { n + base.modulus() } else { n };
    let digits = match base {
        Base::Bin => format!("{raw:b}"),
        Base::Oct => format!("{raw:o}"),
        Base::Hex => format!("{raw:X}"),
    };
    let Some(places) = places else {
        return Ok(Value::from(digits));
    };
    let places = places.trunc();
    if !(1.0..=10.0).contains(&places) {
        return Err(ErrorKind::Num);
    }
    if n < 0 {
        return Ok(Value::from(digits));
    }
    let places = places as usize;
    if digits.len() > places {
        return Err(ErrorKind::Num);
    }
    Ok(Value::from(format!("{digits:0>places$}")))
}

fn places_arg(ctx: &EvalCtx, args: &[Expr]) -> R<Option<f64>> {
    match args.get(1) {
        None | Some(Expr::Missing) => Ok(None),
        Some(e) => number(ctx, e).map(Some),
    }
}

/// `DEC2BIN` and friends.
fn from_decimal(ctx: &EvalCtx, args: &[Expr], to: Base) -> R<Value> {
    let n = number(ctx, &args[0])?.trunc();
    let places = places_arg(ctx, args)?;
    if n < to.min() as f64 || n > to.max() as f64 {
        return Err(ErrorKind::Num);
    }
    format_base(n as i64, to, places)
}

/// `BIN2DEC` and friends.
fn to_decimal(ctx: &EvalCtx, args: &[Expr], from: Base) -> R<Value> {
    let n = parse_base(&digits_arg(ctx, &args[0])?, from)?;
    Ok(Value::Number(n as f64))
}

/// `BIN2HEX` and friends.
fn convert_base(ctx: &EvalCtx, args: &[Expr], from: Base, to: Base) -> R<Value> {
    let n = parse_base(&digits_arg(ctx, &args[0])?, from)?;
    let places = places_arg(ctx, args)?;
    format_base(n, to, places)
}

fn f_dec2bin(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    from_decimal(ctx, args, Base::Bin)
}

fn f_dec2oct(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    from_decimal(ctx, args, Base::Oct)
}

fn f_dec2hex(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    from_decimal(ctx, args, Base::Hex)
}

fn f_bin2dec(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    to_decimal(ctx, args, Base::Bin)
}

fn f_oct2dec(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    to_decimal(ctx, args, Base::Oct)
}

fn f_hex2dec(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    to_decimal(ctx, args, Base::Hex)
}

fn f_bin2oct(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Bin, Base::Oct)
}

fn f_bin2hex(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Bin, Base::Hex)
}

fn f_oct2bin(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Oct, Base::Bin)
}

fn f_oct2hex(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Oct, Base::Hex)
}

fn f_hex2bin(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Hex, Base::Bin)
}

fn f_hex2oct(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    convert_base(ctx, args, Base::Hex, Base::Oct)
}

// ---- bit operations ------------------------------------------------------

/// Largest operand and result of the bit functions: 2^48 - 1.
const BIT_MAX: f64 = 281_474_976_710_655.0;

/// A non-negative integer below 2^48.
fn bit_arg(ctx: &EvalCtx, e: &Expr) -> R<u64> {
    let n = number(ctx, e)?;
    if n < 0.0 || n > BIT_MAX || n.fract() != 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(n as u64)
}

fn bit_op(ctx: &EvalCtx, args: &[Expr], op: fn(u64, u64) -> u64) -> R<Value> {
    let a = bit_arg(ctx, &args[0])?;
    let b = bit_arg(ctx, &args[1])?;
    Ok(Value::Number(op(a, b) as f64))
}

fn f_bitand(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    bit_op(ctx, args, |a, b| a & b)
}

fn f_bitor(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    bit_op(ctx, args, |a, b| a | b)
}

fn f_bitxor(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    bit_op(ctx, args, |a, b| a ^ b)
}

/// Shift left by `shift` bits (right when negative).
fn bit_shift(ctx: &EvalCtx, args: &[Expr], left: bool) -> R<Value> {
    let n = bit_arg(ctx, &args[0])?;
    let shift = number(ctx, &args[1])?.trunc();
    if shift.abs() > 53.0 {
        return Err(ErrorKind::Num);
    }
    let shift = if left { shift as i32 } else { -(shift as i32) };
    let result = if shift >= 0 {
        // Up to 48 + 53 bits: widen so nothing is lost before the check.
        (n as u128) << shift
    } else {
        (n as u128) >> -shift
    };
    if result > BIT_MAX as u128 {
        return Err(ErrorKind::Num);
    }
    Ok(Value::Number(result as f64))
}

fn f_bitlshift(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    bit_shift(ctx, args, true)
}

fn f_bitrshift(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    bit_shift(ctx, args, false)
}

// ---- comparisons ---------------------------------------------------------

fn f_delta(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let a = number(ctx, &args[0])?;
    let b = number_or(ctx, args, 1, 0.0)?;
    Ok(Value::Number(if a == b { 1.0 } else { 0.0 }))
}

fn f_gestep(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = number(ctx, &args[0])?;
    let step = number_or(ctx, args, 1, 0.0)?;
    Ok(Value::Number(if n >= step { 1.0 } else { 0.0 }))
}

// ---- CONVERT -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Quantity {
    Mass,
    Length,
    Time,
    Pressure,
    Force,
    Energy,
    Power,
    Magnetism,
    Temperature,
    Volume,
    Area,
    Information,
    Speed,
}

/// Which multiplier prefixes a unit accepts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prefix {
    None,
    /// SI prefixes (k, m, µ ...).
    Metric,
    /// SI and binary (ki, Mi ...) prefixes.
    Binary,
}

struct Unit {
    name: &'static str,
    quantity: Quantity,
    /// Size in the quantity's base unit (unused for temperatures).
    factor: f64,
    prefix: Prefix,
    /// Exponent applied to a prefix: 2 for areas, 3 for volumes.
    power: i32,
}

const fn unit(name: &'static str, quantity: Quantity, factor: f64) -> Unit {
    Unit {
        name,
        quantity,
        factor,
        prefix: Prefix::None,
        power: 1,
    }
}

const fn metric(name: &'static str, quantity: Quantity, factor: f64) -> Unit {
    Unit {
        name,
        quantity,
        factor,
        prefix: Prefix::Metric,
        power: 1,
    }
}

const fn metric_pow(name: &'static str, quantity: Quantity, factor: f64, power: i32) -> Unit {
    Unit {
        name,
        quantity,
        factor,
        prefix: Prefix::Metric,
        power,
    }
}

/// One typographic point (1/72 inch) in metres.
const POINT: f64 = 0.0254 / 72.0;

use Quantity::*;

/// Unit names are case-sensitive. Base units: gram, metre, second, pascal,
/// newton, joule, watt, tesla, litre, square metre, bit, metre per second.
static UNITS: &[Unit] = &[
    // Weight and mass
    metric("g", Mass, 1.0),
    unit("sg", Mass, 14_593.902_937_206_4),
    unit("lbm", Mass, 453.592_37),
    metric("u", Mass, 1.660_538_782e-24),
    unit("ozm", Mass, 28.349_523_125),
    unit("grain", Mass, 0.064_798_91),
    unit("cwt", Mass, 45_359.237),
    unit("shweight", Mass, 45_359.237),
    unit("uk_cwt", Mass, 50_802.345_44),
    unit("lcwt", Mass, 50_802.345_44),
    unit("hweight", Mass, 50_802.345_44),
    unit("stone", Mass, 6_350.293_18),
    unit("ton", Mass, 907_184.74),
    unit("uk_ton", Mass, 1_016_046.908_8),
    unit("LTON", Mass, 1_016_046.908_8),
    unit("brton", Mass, 1_016_046.908_8),
    // Distance
    metric("m", Length, 1.0),
    unit("mi", Length, 1_609.344),
    unit("Nmi", Length, 1_852.0),
    unit("in", Length, 0.0254),
    unit("ft", Length, 0.3048),
    unit("yd", Length, 0.9144),
    metric("ang", Length, 1e-10),
    unit("ell", Length, 1.143),
    metric("ly", Length, 9.460_730_472_580_8e15),
    metric("parsec", Length, 3.085_677_581_281_55e16),
    metric("pc", Length, 3.085_677_581_281_55e16),
    unit("Picapt", Length, POINT),
    unit("Pica", Length, POINT),
    unit("pica", Length, 0.0254 / 6.0),
    unit("survey_mi", Length, 1_609.347_218_694_44),
    // Time
    unit("yr", Time, 31_557_600.0),
    unit("day", Time, 86_400.0),
    unit("d", Time, 86_400.0),
    unit("hr", Time, 3_600.0),
    unit("mn", Time, 60.0),
    unit("min", Time, 60.0),
    metric("sec", Time, 1.0),
    metric("s", Time, 1.0),
    // Pressure
    metric("Pa", Pressure, 1.0),
    metric("p", Pressure, 1.0),
    metric("atm", Pressure, 101_325.0),
    metric("at", Pressure, 101_325.0),
    metric("mmHg", Pressure, 133.322),
    unit("psi", Pressure, 6_894.757_293_168_36),
    unit("Torr", Pressure, 133.322_368_421_053),
    // Force
    metric("N", Force, 1.0),
    metric("dyn", Force, 1e-5),
    metric("dy", Force, 1e-5),
    unit("lbf", Force, 4.448_221_615_260_5),
    metric("pond", Force, 0.009_806_65),
    // Energy
    metric("J", Energy, 1.0),
    metric("e", Energy, 1e-7),
    metric("c", Energy, 4.184),
    metric("cal", Energy, 4.186_8),
    metric("eV", Energy, 1.602_176_487e-19),
    metric("ev", Energy, 1.602_176_487e-19),
    unit("HPh", Energy, 2_684_519.537_696_17),
    unit("hh", Energy, 2_684_519.537_696_17),
    metric("Wh", Energy, 3_600.0),
    metric("wh", Energy, 3_600.0),
    unit("flb", Energy, 1.355_817_948_331_4),
    unit("BTU", Energy, 1_055.055_852_62),
    unit("btu", Energy, 1_055.055_852_62),
    // Power
    unit("HP", Power, 745.699_871_582_27),
    unit("h", Power, 745.699_871_582_27),
    unit("PS", Power, 735.498_75),
    metric("W", Power, 1.0),
    metric("w", Power, 1.0),
    // Magnetism
    metric("T", Magnetism, 1.0),
    metric("ga", Magnetism, 1e-4),
    // Temperature (converted by formula, see `to_celsius`)
    unit("C", Temperature, 1.0),
    unit("cel", Temperature, 1.0),
    unit("F", Temperature, 1.0),
    unit("fah", Temperature, 1.0),
    unit("K", Temperature, 1.0),
    unit("kel", Temperature, 1.0),
    unit("Rank", Temperature, 1.0),
    unit("Reau", Temperature, 1.0),
    // Volume
    unit("tsp", Volume, 0.004_928_921_593_75),
    unit("tspm", Volume, 0.005),
    unit("tbs", Volume, 0.014_786_764_781_25),
    unit("oz", Volume, 0.029_573_529_562_5),
    unit("cup", Volume, 0.236_588_236_5),
    unit("pt", Volume, 0.473_176_473),
    unit("us_pt", Volume, 0.473_176_473),
    unit("uk_pt", Volume, 0.568_261_25),
    unit("qt", Volume, 0.946_352_946),
    unit("uk_qt", Volume, 1.136_522_5),
    unit("gal", Volume, 3.785_411_784),
    unit("uk_gal", Volume, 4.546_09),
    metric("l", Volume, 1.0),
    metric("L", Volume, 1.0),
    metric("lt", Volume, 1.0),
    metric_pow("ang3", Volume, 1e-27, 3),
    unit("barrel", Volume, 158.987_294_928),
    unit("bushel", Volume, 35.239_070_166_88),
    unit("ft3", Volume, 28.316_846_592),
    unit("ft^3", Volume, 28.316_846_592),
    unit("in3", Volume, 0.016_387_064),
    unit("in^3", Volume, 0.016_387_064),
    metric_pow("ly3", Volume, 8.467_866_646_237_15e50, 3),
    metric_pow("m3", Volume, 1_000.0, 3),
    metric_pow("m^3", Volume, 1_000.0, 3),
    unit("mi3", Volume, 4.168_181_825_440_58e12),
    unit("mi^3", Volume, 4.168_181_825_440_58e12),
    unit("yd3", Volume, 764.554_857_984),
    unit("yd^3", Volume, 764.554_857_984),
    unit("Nmi3", Volume, 6.350_182_608e12),
    unit("Picapt3", Volume, POINT * POINT * POINT * 1_000.0),
    unit("Pica3", Volume, POINT * POINT * POINT * 1_000.0),
    unit("GRT", Volume, 2_831.684_659_2),
    unit("regton", Volume, 2_831.684_659_2),
    unit("MTON", Volume, 1_132.673_863_68),
    // Area
    unit("uk_acre", Area, 4_046.856_422_4),
    unit("us_acre", Area, 4_046.872_609_874_25),
    metric_pow("ang2", Area, 1e-20, 2),
    metric("ar", Area, 100.0),
    unit("ft2", Area, 0.092_903_04),
    unit("ft^2", Area, 0.092_903_04),
    unit("ha", Area, 10_000.0),
    unit("in2", Area, 0.000_645_16),
    unit("in^2", Area, 0.000_645_16),
    metric_pow("ly2", Area, 8.950_542_107_481_89e31, 2),
    metric_pow("m2", Area, 1.0, 2),
    metric_pow("m^2", Area, 1.0, 2),
    unit("Morgen", Area, 2_500.0),
    unit("mi2", Area, 2_589_988.110_336),
    unit("mi^2", Area, 2_589_988.110_336),
    unit("Nmi2", Area, 3_429_904.0),
    unit("Picapt2", Area, POINT * POINT),
    unit("Pica2", Area, POINT * POINT),
    unit("yd2", Area, 0.836_127_36),
    unit("yd^2", Area, 0.836_127_36),
    // Information
    Unit {
        name: "bit",
        quantity: Information,
        factor: 1.0,
        prefix: Prefix::Binary,
        power: 1,
    },
    Unit {
        name: "byte",
        quantity: Information,
        factor: 8.0,
        prefix: Prefix::Binary,
        power: 1,
    },
    // Speed
    unit("admkn", Speed, 0.514_773_333_333_333),
    unit("kn", Speed, 0.514_444_444_444_444),
    metric("m/h", Speed, 1.0 / 3_600.0),
    metric("m/hr", Speed, 1.0 / 3_600.0),
    metric("m/s", Speed, 1.0),
    metric("m/sec", Speed, 1.0),
    unit("mph", Speed, 0.447_04),
];

const METRIC_PREFIXES: &[(&str, f64)] = &[
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("da", 1e1),
    ("e", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
];

const BINARY_PREFIXES: &[(&str, f64)] = &[
    ("Yi", 1_208_925_819_614_629_174_706_176.0),
    ("Zi", 1_180_591_620_717_411_303_424.0),
    ("Ei", 1_152_921_504_606_846_976.0),
    ("Pi", 1_125_899_906_842_624.0),
    ("Ti", 1_099_511_627_776.0),
    ("Gi", 1_073_741_824.0),
    ("Mi", 1_048_576.0),
    ("ki", 1_024.0),
];

/// Resolve a unit name, possibly carrying a prefix, to the unit and the
/// multiplier of the prefix.
fn find_unit(name: &str) -> Option<(&'static Unit, f64)> {
    if let Some(u) = UNITS.iter().find(|u| u.name == name) {
        return Some((u, 1.0));
    }
    let with_prefix = |prefixes: &[(&str, f64)], binary: bool| {
        prefixes.iter().find_map(|(prefix, scale)| {
            let rest = name.strip_prefix(*prefix)?;
            let u = UNITS.iter().find(|u| u.name == rest)?;
            let allowed = match u.prefix {
                Prefix::None => false,
                Prefix::Metric => !binary,
                Prefix::Binary => true,
            };
            allowed.then(|| (u, scale.powi(u.power)))
        })
    };
    with_prefix(BINARY_PREFIXES, true).or_else(|| with_prefix(METRIC_PREFIXES, false))
}

fn to_celsius(x: f64, unit: &str) -> f64 {
    match unit {
        "F" | "fah" => (x - 32.0) / 1.8,
        "K" | "kel" => x - 273.15,
        "Rank" => (x - 491.67) / 1.8,
        "Reau" => x * 1.25,
        _ => x,
    }
}

fn from_celsius(c: f64, unit: &str) -> f64 {
    match unit {
        "F" | "fah" => c * 1.8 + 32.0,
        "K" | "kel" => c + 273.15,
        "Rank" => (c + 273.15) * 1.8,
        "Reau" => c * 0.8,
        _ => c,
    }
}

fn f_convert(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = number(ctx, &args[0])?;
    let from_name = ctx.text(&args[1])?;
    let to_name = ctx.text(&args[2])?;
    let (from, from_scale) = find_unit(&from_name).ok_or(ErrorKind::NA)?;
    let (to, to_scale) = find_unit(&to_name).ok_or(ErrorKind::NA)?;
    if from.quantity != to.quantity {
        return Err(ErrorKind::NA);
    }
    let result = if from.quantity == Temperature {
        if from.name == to.name {
            x
        } else {
            from_celsius(to_celsius(x, from.name), to.name)
        }
    } else {
        x * (from.factor * from_scale) / (to.factor * to_scale)
    };
    if result.is_finite() {
        Ok(Value::Number(result))
    } else {
        Err(ErrorKind::Num)
    }
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("DEC2BIN", 1, 2, Kind::Scalar, Category::Engineering, f_dec2bin),
    FunctionDef::new("DEC2OCT", 1, 2, Kind::Scalar, Category::Engineering, f_dec2oct),
    FunctionDef::new("DEC2HEX", 1, 2, Kind::Scalar, Category::Engineering, f_dec2hex),
    FunctionDef::new("BIN2DEC", 1, 1, Kind::Scalar, Category::Engineering, f_bin2dec),
    FunctionDef::new("OCT2DEC", 1, 1, Kind::Scalar, Category::Engineering, f_oct2dec),
    FunctionDef::new("HEX2DEC", 1, 1, Kind::Scalar, Category::Engineering, f_hex2dec),
    FunctionDef::new("BIN2OCT", 1, 2, Kind::Scalar, Category::Engineering, f_bin2oct),
    FunctionDef::new("BIN2HEX", 1, 2, Kind::Scalar, Category::Engineering, f_bin2hex),
    FunctionDef::new("OCT2BIN", 1, 2, Kind::Scalar, Category::Engineering, f_oct2bin),
    FunctionDef::new("OCT2HEX", 1, 2, Kind::Scalar, Category::Engineering, f_oct2hex),
    FunctionDef::new("HEX2BIN", 1, 2, Kind::Scalar, Category::Engineering, f_hex2bin),
    FunctionDef::new("HEX2OCT", 1, 2, Kind::Scalar, Category::Engineering, f_hex2oct),
    FunctionDef::new("BITAND", 2, 2, Kind::Scalar, Category::Engineering, f_bitand),
    FunctionDef::new("BITOR", 2, 2, Kind::Scalar, Category::Engineering, f_bitor),
    FunctionDef::new("BITXOR", 2, 2, Kind::Scalar, Category::Engineering, f_bitxor),
    FunctionDef::new("BITLSHIFT", 2, 2, Kind::Scalar, Category::Engineering, f_bitlshift),
    FunctionDef::new("BITRSHIFT", 2, 2, Kind::Scalar, Category::Engineering, f_bitrshift),
    FunctionDef::new("DELTA", 1, 2, Kind::Scalar, Category::Engineering, f_delta),
    FunctionDef::new("GESTEP", 1, 2, Kind::Scalar, Category::Engineering, f_gestep),
    FunctionDef::new("CONVERT", 3, 3, Kind::Scalar, Category::Engineering, f_convert),
];

#[cfg(test)]
mod tests {
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};

    fn text(formula: &str) -> String {
        match eval(formula) {
            Value::Text(s) => s.to_string(),
            other => panic!("{formula} gave {other:?}, expected text"),
        }
    }

    fn err(formula: &str) -> ErrorKind {
        match eval(formula) {
            Value::Error(e) => e,
            other => panic!("{formula} gave {other:?}, expected an error"),
        }
    }

    fn close(formula: &str, expected: f64) {
        let got = num(formula);
        let tol = 1e-9 * expected.abs().max(1e-6);
        assert!(
            (got - expected).abs() <= tol,
            "{formula} gave {got}, expected {expected}"
        );
    }

    #[test]
    fn from_decimal() {
        assert_eq!(text("DEC2BIN(9)"), "1001");
        assert_eq!(text("DEC2BIN(9,4)"), "1001");
        assert_eq!(text("DEC2BIN(9,8)"), "00001001");
        assert_eq!(text("DEC2BIN(0)"), "0");
        assert_eq!(text("DEC2BIN(511)"), "111111111");
        assert_eq!(text("DEC2BIN(-1)"), "1111111111");
        assert_eq!(text("DEC2BIN(-100)"), "1110011100");
        assert_eq!(text("DEC2BIN(-512)"), "1000000000");
        assert_eq!(text("DEC2BIN(-3,4)"), "1111111101");
        assert_eq!(text("DEC2BIN(9.9)"), "1001");
        assert_eq!(text("DEC2BIN(\"9\")"), "1001");
        assert_eq!(err("DEC2BIN(512)"), ErrorKind::Num);
        assert_eq!(err("DEC2BIN(-513)"), ErrorKind::Num);
        assert_eq!(err("DEC2BIN(9,3)"), ErrorKind::Num);
        assert_eq!(err("DEC2BIN(9,0)"), ErrorKind::Num);
        assert_eq!(err("DEC2BIN(9,11)"), ErrorKind::Num);
        assert_eq!(err("DEC2BIN(\"x\")"), ErrorKind::Value);
        assert_eq!(err("DEC2BIN(TRUE)"), ErrorKind::Value);
        assert_eq!(err("DEC2BIN(9,\"x\")"), ErrorKind::Value);
        assert_eq!(text("DEC2HEX(100,4)"), "0064");
        assert_eq!(text("DEC2HEX(255)"), "FF");
        assert_eq!(text("DEC2HEX(-54)"), "FFFFFFFFCA");
        assert_eq!(text("DEC2HEX(28)"), "1C");
        assert_eq!(text("DEC2HEX(549755813887)"), "7FFFFFFFFF");
        assert_eq!(text("DEC2HEX(-549755813888)"), "8000000000");
        assert_eq!(err("DEC2HEX(549755813888)"), ErrorKind::Num);
        assert_eq!(err("DEC2HEX(64,1)"), ErrorKind::Num);
        assert_eq!(text("DEC2OCT(58,3)"), "072");
        assert_eq!(text("DEC2OCT(-100)"), "7777777634");
        assert_eq!(text("DEC2OCT(536870911)"), "3777777777");
        assert_eq!(text("DEC2OCT(-536870912)"), "4000000000");
        assert_eq!(err("DEC2OCT(536870912)"), ErrorKind::Num);
    }

    #[test]
    fn to_decimal() {
        assert_eq!(num("BIN2DEC(\"1100100\")"), 100.0);
        assert_eq!(num("BIN2DEC(1100100)"), 100.0);
        assert_eq!(num("BIN2DEC(\"1111111111\")"), -1.0);
        assert_eq!(num("BIN2DEC(1111111111)"), -1.0);
        assert_eq!(num("BIN2DEC(\"1000000000\")"), -512.0);
        assert_eq!(num("BIN2DEC(\"0111111111\")"), 511.0);
        assert_eq!(num("BIN2DEC(\"\")"), 0.0);
        assert_eq!(err("BIN2DEC(\"102\")"), ErrorKind::Num);
        assert_eq!(err("BIN2DEC(\"11111111111\")"), ErrorKind::Num);
        assert_eq!(err("BIN2DEC(-1)"), ErrorKind::Num);
        assert_eq!(err("BIN2DEC(1.5)"), ErrorKind::Num);
        assert_eq!(err("BIN2DEC(TRUE)"), ErrorKind::Value);
        assert_eq!(num("HEX2DEC(\"A5\")"), 165.0);
        assert_eq!(num("HEX2DEC(\"ff\")"), 255.0);
        assert_eq!(num("HEX2DEC(10)"), 16.0);
        assert_eq!(num("HEX2DEC(\"FFFFFFFF5B\")"), -165.0);
        assert_eq!(num("HEX2DEC(\"3DA408B9\")"), 1034160313.0);
        assert_eq!(num("HEX2DEC(\"7FFFFFFFFF\")"), 549755813887.0);
        assert_eq!(num("HEX2DEC(\"8000000000\")"), -549755813888.0);
        assert_eq!(err("HEX2DEC(\"G1\")"), ErrorKind::Num);
        assert_eq!(err("HEX2DEC(\"12345678901\")"), ErrorKind::Num);
        assert_eq!(num("OCT2DEC(54)"), 44.0);
        assert_eq!(num("OCT2DEC(\"7777777533\")"), -165.0);
        assert_eq!(num("OCT2DEC(\"3777777777\")"), 536870911.0);
        assert_eq!(err("OCT2DEC(\"8\")"), ErrorKind::Num);
        assert_eq!(err("OCT2DEC(1/0)"), ErrorKind::Div0);
    }

    #[test]
    fn between_bases() {
        assert_eq!(text("BIN2HEX(\"11111011\",4)"), "00FB");
        assert_eq!(text("BIN2HEX(\"1110\")"), "E");
        assert_eq!(text("BIN2HEX(\"1111111111\")"), "FFFFFFFFFF");
        assert_eq!(text("BIN2OCT(\"1001\",3)"), "011");
        assert_eq!(text("BIN2OCT(\"1100100\")"), "144");
        assert_eq!(text("BIN2OCT(\"1111111111\")"), "7777777777");
        assert_eq!(text("HEX2BIN(\"F\",8)"), "00001111");
        assert_eq!(text("HEX2BIN(\"B7\")"), "10110111");
        assert_eq!(text("HEX2BIN(\"FFFFFFFFFF\")"), "1111111111");
        assert_eq!(text("HEX2BIN(\"1FF\")"), "111111111");
        assert_eq!(err("HEX2BIN(\"200\")"), ErrorKind::Num);
        assert_eq!(err("HEX2BIN(\"F\",3)"), ErrorKind::Num);
        assert_eq!(text("HEX2OCT(\"F\",3)"), "017");
        assert_eq!(text("HEX2OCT(\"3B4E\")"), "35516");
        assert_eq!(text("HEX2OCT(\"FFFFFFFF00\")"), "7777777400");
        assert_eq!(err("HEX2OCT(\"FFFFFFFFF\")"), ErrorKind::Num);
        assert_eq!(text("OCT2BIN(3,3)"), "011");
        assert_eq!(text("OCT2BIN(\"7777777000\")"), "1000000000");
        assert_eq!(err("OCT2BIN(\"1000\")"), ErrorKind::Num);
        assert_eq!(text("OCT2HEX(100,4)"), "0040");
        assert_eq!(text("OCT2HEX(\"7777777533\")"), "FFFFFFFF5B");
        assert_eq!(err("OCT2HEX(\"9\")"), ErrorKind::Num);
        // Lifted over a range.
        let w = wb(&[("A1", "5"), ("A2", "10"), ("C1", "=DEC2BIN(A1:A2,4)")]);
        assert_eq!(val(&w, "C1"), Value::text("0101"));
        assert_eq!(val(&w, "C2"), Value::text("1010"));
    }

    #[test]
    fn bits() {
        assert_eq!(num("BITAND(1,5)"), 1.0);
        assert_eq!(num("BITAND(13,25)"), 9.0);
        assert_eq!(num("BITOR(23,10)"), 31.0);
        assert_eq!(num("BITXOR(5,3)"), 6.0);
        assert_eq!(num("BITAND(281474976710655,255)"), 255.0);
        assert_eq!(num("BITOR(\"4\",1)"), 5.0);
        assert_eq!(err("BITAND(-1,5)"), ErrorKind::Num);
        assert_eq!(err("BITAND(1.5,5)"), ErrorKind::Num);
        assert_eq!(err("BITOR(281474976710656,1)"), ErrorKind::Num);
        assert_eq!(err("BITXOR(\"a\",1)"), ErrorKind::Value);
        assert_eq!(err("BITXOR(TRUE,1)"), ErrorKind::Value);
        assert_eq!(num("BITLSHIFT(4,2)"), 16.0);
        assert_eq!(num("BITRSHIFT(13,2)"), 3.0);
        assert_eq!(num("BITLSHIFT(13,-2)"), 3.0);
        assert_eq!(num("BITRSHIFT(4,-2)"), 16.0);
        assert_eq!(num("BITLSHIFT(1,47)"), 140737488355328.0);
        assert_eq!(num("BITRSHIFT(1,53)"), 0.0);
        assert_eq!(num("BITLSHIFT(0,53)"), 0.0);
        assert_eq!(err("BITLSHIFT(1,48)"), ErrorKind::Num);
        assert_eq!(err("BITLSHIFT(1,54)"), ErrorKind::Num);
        assert_eq!(err("BITRSHIFT(1,-54)"), ErrorKind::Num);
        assert_eq!(err("BITLSHIFT(-1,1)"), ErrorKind::Num);
        assert_eq!(err("BITRSHIFT(8,\"x\")"), ErrorKind::Value);
    }

    #[test]
    fn delta_and_gestep() {
        assert_eq!(num("DELTA(5,4)"), 0.0);
        assert_eq!(num("DELTA(5,5)"), 1.0);
        assert_eq!(num("DELTA(0.5,0)"), 0.0);
        assert_eq!(num("DELTA(0)"), 1.0);
        assert_eq!(num("DELTA(\"3\",3)"), 1.0);
        assert_eq!(err("DELTA(\"a\",3)"), ErrorKind::Value);
        assert_eq!(err("DELTA(TRUE,1)"), ErrorKind::Value);
        assert_eq!(num("GESTEP(5,4)"), 1.0);
        assert_eq!(num("GESTEP(5,5)"), 1.0);
        assert_eq!(num("GESTEP(-4,-5)"), 1.0);
        assert_eq!(num("GESTEP(-1)"), 0.0);
        assert_eq!(num("GESTEP(0)"), 1.0);
        assert_eq!(err("GESTEP(\"a\")"), ErrorKind::Value);
        let w = wb(&[("A1", "1"), ("A2", "-2"), ("C1", "=GESTEP(A1:A2)")]);
        assert_eq!(val(&w, "C1"), Value::Number(1.0));
        assert_eq!(val(&w, "C2"), Value::Number(0.0));
    }

    #[test]
    fn convert_lengths_and_masses() {
        close("CONVERT(1,\"mi\",\"km\")", 1.609344);
        close("CONVERT(1,\"km\",\"m\")", 1000.0);
        close("CONVERT(1,\"m\",\"cm\")", 100.0);
        close("CONVERT(25.4,\"mm\",\"in\")", 1.0);
        close("CONVERT(1,\"ft\",\"in\")", 12.0);
        close("CONVERT(1,\"yd\",\"ft\")", 3.0);
        close("CONVERT(100,\"ft\",\"m\")", 30.48);
        close("CONVERT(1,\"mi\",\"yd\")", 1760.0);
        close("CONVERT(6,\"m\",\"m\")", 6.0);
        close("CONVERT(1,\"lbm\",\"kg\")", 0.45359237);
        close("CONVERT(1,\"kg\",\"g\")", 1000.0);
        close("CONVERT(1,\"lbm\",\"ozm\")", 16.0);
        close("CONVERT(1,\"ozm\",\"g\")", 28.349523125);
        close("CONVERT(5,\"mg\",\"g\")", 0.005);
        close("CONVERT(-2,\"kg\",\"lbm\")", -4.409245243697551);
        close("CONVERT(\"3\",\"m\",\"mm\")", 3000.0);
    }

    #[test]
    fn convert_time_temperature_volume() {
        close("CONVERT(1,\"day\",\"hr\")", 24.0);
        close("CONVERT(1,\"hr\",\"min\")", 60.0);
        close("CONVERT(1,\"hr\",\"mn\")", 60.0);
        close("CONVERT(1,\"min\",\"sec\")", 60.0);
        close("CONVERT(1,\"yr\",\"day\")", 365.25);
        close("CONVERT(90,\"sec\",\"min\")", 1.5);
        close("CONVERT(1500,\"ms\",\"s\")", 1.5);
        assert_eq!(num("CONVERT(68,\"F\",\"C\")"), 20.0);
        assert_eq!(num("CONVERT(100,\"C\",\"F\")"), 212.0);
        assert_eq!(num("CONVERT(100,\"C\",\"K\")"), 373.15);
        close("CONVERT(0,\"K\",\"C\")", -273.15);
        close("CONVERT(32,\"F\",\"K\")", 273.15);
        close("CONVERT(-40,\"C\",\"F\")", -40.0);
        close("CONVERT(300,\"K\",\"F\")", 80.33);
        assert_eq!(num("CONVERT(21.5,\"K\",\"K\")"), 21.5);
        close("CONVERT(1,\"gal\",\"l\")", 3.785411784);
        close("CONVERT(1,\"gal\",\"qt\")", 4.0);
        close("CONVERT(1,\"qt\",\"pt\")", 2.0);
        close("CONVERT(1,\"pt\",\"cup\")", 2.0);
        close("CONVERT(1,\"cup\",\"tbs\")", 16.0);
        close("CONVERT(1,\"tbs\",\"tsp\")", 3.0);
        close("CONVERT(500,\"ml\",\"l\")", 0.5);
        close("CONVERT(1,\"l\",\"cup\")", 4.226752837730375);
    }

    #[test]
    fn convert_other_and_errors() {
        close("CONVERT(1,\"m3\",\"l\")", 1000.0);
        close("CONVERT(1,\"km2\",\"ha\")", 100.0);
        close("CONVERT(1,\"ha\",\"m2\")", 10000.0);
        close("CONVERT(1,\"atm\",\"Pa\")", 101325.0);
        close("CONVERT(1,\"kWh\",\"J\")", 3600000.0);
        close("CONVERT(1,\"HP\",\"W\")", 745.69987158227);
        close("CONVERT(1,\"byte\",\"bit\")", 8.0);
        close("CONVERT(1,\"kibyte\",\"byte\")", 1024.0);
        close("CONVERT(1,\"kbyte\",\"byte\")", 1000.0);
        close("CONVERT(60,\"mph\",\"m/s\")", 26.8224);
        close("CONVERT(1,\"N\",\"dyn\")", 100000.0);
        assert_eq!(err("CONVERT(2.5,\"ft\",\"sec\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(1,\"C\",\"m\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(1,\"furlong\",\"m\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(1,\"m\",\"\")"), ErrorKind::NA);
        // Unit names are case-sensitive, and only some take prefixes.
        assert_eq!(err("CONVERT(1,\"KM\",\"m\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(1,\"kft\",\"m\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(1,\"kim\",\"m\")"), ErrorKind::NA);
        assert_eq!(err("CONVERT(\"x\",\"m\",\"km\")"), ErrorKind::Value);
        assert_eq!(err("CONVERT(TRUE,\"m\",\"km\")"), ErrorKind::Value);
        assert_eq!(err("CONVERT(#DIV/0!,\"m\",\"km\")"), ErrorKind::Div0);
    }
}
