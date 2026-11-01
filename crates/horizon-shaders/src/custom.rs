//! Custom uniforms and variables of `shaders.properties` (spec 3.4): `uniform.<type>.<name>` and
//! `variable.<type>.<name>` expressions, parsed once and evaluated on the CPU every frame in
//! file order. Uniforms reach every program; variables only later expressions.
//!
//! Decisions (impl-plan.md):
//! - OQ-25: numbers are doubles; booleans are 1 and 0; `%` is C's remainder (`fmod` floors);
//!   comparisons and logic read the first component; a value takes its declared type at the
//!   end (`int` truncates toward zero, `bool` is "not zero"). Unknown names read 0 and are
//!   reported once.
//! - OQ-26: `smooth` runs per frame with the frame time; its fade times are seconds (1 by
//!   default) to cover half of the remaining distance, `fadeIn` while rising, `fadeOut` while
//!   falling. Each call keeps its own state, so the optional id is not needed.

use std::collections::{HashMap, HashSet};

use crate::Properties;

/// A value: up to 16 components (matrices only as names, for `m.column.row`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Val {
    pub v: [f64; 16],
    /// Components: 1 for a scalar.
    pub n: usize,
}

impl Val {
    pub fn scalar(x: f64) -> Self {
        let mut v = [0.0; 16];
        v[0] = x;
        Self { v, n: 1 }
    }
    pub fn vector(c: &[f64]) -> Self {
        let mut v = [0.0; 16];
        v[..c.len()].copy_from_slice(c);
        Self { v, n: c.len() }
    }
    fn x(&self) -> f64 {
        self.v[0]
    }
    fn truth(&self) -> bool {
        self.v[0] != 0.0
    }
    /// `f` per component; a scalar operand stretches to the other's size.
    fn zip(self, o: Val, f: impl Fn(f64, f64) -> f64) -> Val {
        let n = self.n.max(o.n).min(4);
        let at = |a: &Val, i: usize| if a.n == 1 { a.v[0] } else { a.v[i] };
        let mut out = Val::scalar(0.0);
        out.n = n;
        for i in 0..n {
            out.v[i] = f(at(&self, i), at(&o, i));
        }
        out
    }
    fn map(self, f: impl Fn(f64) -> f64) -> Val {
        let mut out = self;
        for i in 0..self.n.min(4) {
            out.v[i] = f(self.v[i]);
        }
        out
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    /// A name and its component (`x`, `2`, `2.1` as column 2, row 1).
    Name(String, Option<(usize, Option<usize>)>),
    Call(String, Vec<Expr>, usize),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
}

#[derive(Debug)]
struct Item {
    name: String,
    ty: String,
    uniform: bool,
    expr: Expr,
}

/// The pack's custom uniforms and variables, with the state `smooth` and `random` keep.
#[derive(Debug, Default)]
pub struct Custom {
    items: Vec<Item>,
    /// `smooth` state per call site.
    smooth: HashMap<usize, f64>,
    seed: u64,
    unknown: HashSet<String>,
    /// `BIOME_*`, `CAT_*` and `PPT_*` (crate::biomes).
    constants: HashMap<String, f64>,
    /// Expressions that did not parse, and unknown names, as `key: message`.
    pub notes: Vec<String>,
    calls: usize,
}

impl Custom {
    /// Reads every `uniform.*` and `variable.*` key of `properties`, in file order.
    pub fn parse(properties: &Properties) -> Self {
        let mut out = Custom {
            seed: 0x9e37_79b9_7f4a_7c15,
            constants: crate::biomes::constants(&[]),
            ..Default::default()
        };
        for e in &properties.entries {
            let (uniform, rest) = match e.key.split_once('.') {
                Some(("uniform", r)) => (true, r),
                Some(("variable", r)) => (false, r),
                _ => continue,
            };
            let Some((ty, name)) = rest.split_once('.') else {
                continue;
            };
            if !matches!(ty, "float" | "int" | "bool" | "vec2" | "vec3" | "vec4") {
                out.notes
                    .push(format!("{}: type `{ty}` is not supported", e.key));
                continue;
            }
            match Parser::new(&e.value, &mut out.calls).expr_all() {
                Ok(expr) => out.items.push(Item {
                    name: name.to_owned(),
                    ty: ty.to_owned(),
                    uniform,
                    expr,
                }),
                Err(m) => out.notes.push(format!("{}: {m}", e.key)),
            }
        }
        out
    }

    /// Forgets what `smooth` keeps: each starts again from its next value, as on the first frame
    /// (for runs whose values are not the world's: the load-time dry run, frames before the
    /// camera's biome is known).
    pub fn restart_smoothing(&mut self) {
        self.smooth.clear();
    }

    /// Names the game's biomes (registry identifiers, in id order) for `BIOME_*`.
    pub fn set_biomes(&mut self, biomes: &[&str]) {
        self.constants = crate::biomes::constants(biomes);
    }

    /// The uniforms' names and declared types, in file order.
    pub fn uniforms(&self) -> impl Iterator<Item = (&str, &str)> {
        self.items
            .iter()
            .filter(|i| i.uniform)
            .map(|i| (i.name.as_str(), i.ty.as_str()))
    }

    /// Evaluates everything for a frame `dt` seconds after the last; `builtin` gives the
    /// values of the game's uniforms and constants (`BIOME_*`...). Returns the uniforms'
    /// values, in the order of `uniforms`.
    pub fn eval(&mut self, dt: f64, builtin: &dyn Fn(&str) -> Option<Val>) -> Vec<Val> {
        let mut vars: HashMap<String, Val> = HashMap::new();
        let mut out = Vec::new();
        let items = std::mem::take(&mut self.items);
        for item in &items {
            let v = self.value(&item.expr, dt, &vars, builtin);
            let v = match item.ty.as_str() {
                "int" => Val::scalar(v.x().trunc()),
                "bool" => Val::scalar(f64::from(u8::from(v.truth()))),
                "float" => Val::scalar(v.x()),
                t => {
                    let n = (t.as_bytes()[3] - b'0') as usize;
                    let mut v = v;
                    if v.n == 1 {
                        let x = v.v[0];
                        v.v[1..n].fill(x);
                    }
                    v.n = n;
                    v
                }
            };
            vars.insert(item.name.clone(), v);
            if item.uniform {
                out.push(v);
            }
        }
        self.items = items;
        out
    }

    fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }

    fn value(
        &mut self,
        e: &Expr,
        dt: f64,
        vars: &HashMap<String, Val>,
        builtin: &dyn Fn(&str) -> Option<Val>,
    ) -> Val {
        let ev = |e: &Expr, s: &mut Self| s.value(e, dt, vars, builtin);
        match e {
            Expr::Num(x) => Val::scalar(*x),
            Expr::Name(name, component) => {
                let v = match name.as_str() {
                    "pi" => Some(Val::scalar(std::f64::consts::PI)),
                    "true" => Some(Val::scalar(1.0)),
                    "false" => Some(Val::scalar(0.0)),
                    n => vars
                        .get(n)
                        .copied()
                        .or_else(|| builtin(n))
                        .or_else(|| self.constants.get(n).map(|&x| Val::scalar(x))),
                };
                if v.is_none() && name.starts_with("BIOME_") {
                    if self.unknown.insert(name.clone()) {
                        self.notes
                            .push(format!("`{name}` names no biome; it reads -1"));
                    }
                    return Val::scalar(-1.0);
                }
                let Some(v) = v else {
                    if self.unknown.insert(name.clone()) {
                        self.notes.push(format!("`{name}` is unknown; it reads 0"));
                    }
                    return Val::scalar(0.0);
                };
                match component {
                    None => v,
                    // `m.c.r` is GLSL's m[c][r], stored column after column: packs transform a
                    // view vector to the world as `inverse.0.0 * x + inverse.1.0 * y + ...`.
                    Some((column, Some(row))) => Val::scalar(v.v[column * 4 + row]),
                    Some((i, None)) => Val::scalar(v.v[*i]),
                }
            }
            Expr::Neg(a) => ev(a, self).map(|x| -x),
            Expr::Not(a) => Val::scalar(f64::from(u8::from(!ev(a, self).truth()))),
            Expr::Bin(op, a, b) => {
                let (a, b) = (ev(a, self), ev(b, self));
                let bool_ = |c: bool| Val::scalar(f64::from(u8::from(c)));
                match *op {
                    "+" => a.zip(b, |x, y| x + y),
                    "-" => a.zip(b, |x, y| x - y),
                    "*" => a.zip(b, |x, y| x * y),
                    "/" => a.zip(b, |x, y| x / y),
                    "%" => a.zip(b, |x, y| x % y),
                    "<" => bool_(a.x() < b.x()),
                    "<=" => bool_(a.x() <= b.x()),
                    ">" => bool_(a.x() > b.x()),
                    ">=" => bool_(a.x() >= b.x()),
                    "==" => bool_(a.x() == b.x()),
                    "!=" => bool_(a.x() != b.x()),
                    "&&" => bool_(a.truth() && b.truth()),
                    _ => bool_(a.truth() || b.truth()),
                }
            }
            Expr::Call(f, args, site) => {
                let a: Vec<Val> = args.iter().map(|e| ev(e, self)).collect();
                let arg = |i: usize| a.get(i).copied().unwrap_or(Val::scalar(0.0));
                let x = arg(0).x();
                let one = |g: fn(f64) -> f64| arg(0).map(g);
                match f.as_str() {
                    "sin" => one(f64::sin),
                    "cos" => one(f64::cos),
                    "tan" => one(f64::tan),
                    "asin" => one(f64::asin),
                    "acos" => one(f64::acos),
                    "atan" if a.len() == 2 => Val::scalar(x.atan2(arg(1).x())),
                    "atan" => one(f64::atan),
                    "atan2" => Val::scalar(x.atan2(arg(1).x())),
                    "torad" | "radians" => one(f64::to_radians),
                    "todeg" | "degrees" => one(f64::to_degrees),
                    "min" => a
                        .iter()
                        .copied()
                        .reduce(|p, q| p.zip(q, f64::min))
                        .unwrap_or(arg(0)),
                    "max" => a
                        .iter()
                        .copied()
                        .reduce(|p, q| p.zip(q, f64::max))
                        .unwrap_or(arg(0)),
                    "clamp" => arg(0).zip(arg(1), f64::max).zip(arg(2), f64::min),
                    "abs" => one(f64::abs),
                    "floor" => one(f64::floor),
                    "ceil" => one(f64::ceil),
                    "exp" => one(f64::exp),
                    "exp2" => one(f64::exp2),
                    "exp10" => one(|v| 10f64.powf(v)),
                    "frac" => one(|v| v - v.floor()),
                    "log" if a.len() == 2 => Val::scalar(arg(1).x().log(x)),
                    "log" => one(f64::ln),
                    "log2" => one(f64::log2),
                    "log10" => one(f64::log10),
                    "sqrt" => one(f64::sqrt),
                    "round" => one(f64::round),
                    "signum" | "sign" => one(|v| if v == 0.0 { 0.0 } else { v.signum() }),
                    "pow" => arg(0).zip(arg(1), f64::powf),
                    "fmod" => arg(0).zip(arg(1), |p, q| p - q * (p / q).floor()),
                    "lerp" => arg(1).zip(arg(2), |p, q| p + (q - p) * x),
                    "mix" => arg(0).zip(arg(1), |p, q| p + (q - p) * arg(2).x()),
                    "edge" => Val::scalar(if arg(1).x() < x { 0.0 } else { 1.0 }),
                    "random" if a.len() == 2 => Val::scalar(x + (arg(1).x() - x) * self.random()),
                    "random" => Val::scalar(self.random()),
                    "randomInt" if a.len() == 2 => {
                        Val::scalar((x + (arg(1).x() - x) * self.random()).floor())
                    }
                    "randomInt" => Val::scalar((self.random() * f64::from(i32::MAX)).floor()),
                    "if" | "ifb" => {
                        let mut i = 0;
                        while i + 1 < a.len() {
                            if a[i].truth() {
                                return a[i + 1];
                            }
                            i += 2;
                        }
                        a.last().copied().unwrap_or(Val::scalar(0.0))
                    }
                    "smooth" => {
                        let (val, fade_in, fade_out) = match a.len() {
                            4 => (arg(1).x(), arg(2).x(), arg(3).x()),
                            3 => (x, arg(1).x(), arg(2).x()),
                            2 => (x, arg(1).x(), arg(1).x()),
                            _ => (x, 1.0, 1.0),
                        };
                        let state = self.smooth.entry(*site).or_insert(val);
                        let fade = if val > *state { fade_in } else { fade_out };
                        if fade > 0.0 {
                            *state = val + (*state - val) * 0.5f64.powf(dt / fade);
                        } else {
                            *state = val;
                        }
                        Val::scalar(*state)
                    }
                    "print" => arg(2),
                    "between" => {
                        Val::scalar(f64::from(u8::from(x >= arg(1).x() && x <= arg(2).x())))
                    }
                    "equals" => {
                        Val::scalar(f64::from(u8::from((x - arg(1).x()).abs() <= arg(2).x())))
                    }
                    "in" => Val::scalar(f64::from(u8::from(a[1..].iter().any(|v| v.x() == x)))),
                    "vec2" | "vec3" | "vec4" => {
                        let n = (f.as_bytes()[3] - b'0') as usize;
                        let mut c: Vec<f64> = a.iter().flat_map(|v| v.v[..v.n].to_vec()).collect();
                        if c.len() == 1 {
                            c = vec![c[0]; n];
                        }
                        c.resize(n, 0.0);
                        Val::vector(&c)
                    }
                    _ => {
                        if self.unknown.insert(f.clone()) {
                            self.notes.push(format!("`{f}()` is unknown; it gives 0"));
                        }
                        Val::scalar(0.0)
                    }
                }
            }
        }
    }
}

struct Parser<'a> {
    tokens: Vec<String>,
    at: usize,
    calls: &'a mut usize,
}

const OPERATORS: [&str; 18] = [
    "||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "!", "(", ")", ",", ".",
];

impl<'a> Parser<'a> {
    fn new(text: &str, calls: &'a mut usize) -> Self {
        let mut tokens = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i] as char;
            if c.is_whitespace() {
                i += 1;
            } else if c.is_ascii_digit()
                || (c == '.'
                    && b.get(i + 1).is_some_and(u8::is_ascii_digit)
                    && !tokens
                        .last()
                        .is_some_and(|t: &String| t == "." || is_name(t)))
            {
                let start = i;
                while i < b.len()
                    && (b[i].is_ascii_alphanumeric()
                        || b[i] == b'.'
                        || ((b[i] == b'-' || b[i] == b'+') && matches!(b[i - 1], b'e' | b'E')))
                {
                    i += 1;
                }
                tokens.push(text[start..i].to_owned());
            } else if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                tokens.push(text[start..i].to_owned());
            } else {
                let op = OPERATORS
                    .iter()
                    .find(|o| text[i..].starts_with(**o))
                    .map_or(&text[i..i + c.len_utf8()], |o| o);
                i += op.len();
                tokens.push(op.to_owned());
            }
        }
        Parser {
            tokens,
            at: 0,
            calls,
        }
    }
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }
    fn eat(&mut self, t: &str) -> bool {
        let hit = self.peek() == Some(t);
        self.at += usize::from(hit);
        hit
    }
    fn expr_all(&mut self) -> Result<Expr, String> {
        let e = self.binary(0)?;
        match self.peek() {
            None => Ok(e),
            Some(t) => Err(format!("unexpected `{t}`")),
        }
    }
    /// Precedence levels from the loosest: `||`, `&&`, equality, relations, sums, products.
    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[&str]; 6] = [
            &["||"],
            &["&&"],
            &["==", "!="],
            &["<", "<=", ">", ">="],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self
            .peek()
            .and_then(|t| LEVELS[level].iter().find(|o| **o == t))
        {
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.eat("+");
        self.primary()
    }
    fn primary(&mut self) -> Result<Expr, String> {
        let Some(t) = self.peek().map(str::to_owned) else {
            return Err("the expression ends early".into());
        };
        self.at += 1;
        if t == "(" {
            let e = self.binary(0)?;
            return if self.eat(")") {
                Ok(e)
            } else {
                Err("missing `)`".into())
            };
        }
        if t.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
            return t
                .trim_end_matches(['f', 'F'])
                .parse()
                .map(Expr::Num)
                .map_err(|_| format!("bad number `{t}`"));
        }
        if !is_name(&t) {
            return Err(format!("unexpected `{t}`"));
        }
        if self.eat("(") {
            let mut args = Vec::new();
            if !self.eat(")") {
                loop {
                    args.push(self.binary(0)?);
                    if self.eat(")") {
                        break;
                    }
                    if !self.eat(",") {
                        return Err(format!("missing `)` after the arguments of `{t}`"));
                    }
                }
            }
            *self.calls += 1;
            return Ok(Expr::Call(t, args, *self.calls));
        }
        let component = if self.eat(".") {
            let c = self.peek().map(str::to_owned).ok_or("nothing after `.`")?;
            self.at += 1;
            let index = |s: &str| match s {
                "x" | "r" | "s" | "0" => Some(0),
                "y" | "g" | "t" | "1" => Some(1),
                "z" | "b" | "p" | "2" => Some(2),
                "w" | "a" | "q" | "3" => Some(3),
                _ => None,
            };
            Some(match c.split_once('.') {
                Some((r, col)) => (
                    index(r).ok_or(format!("bad component `{c}`"))?,
                    Some(index(col).ok_or(format!("bad component `{c}`"))?),
                ),
                None => (index(&c).ok_or(format!("bad component `{c}`"))?, None),
            })
        } else {
            None
        };
        Ok(Expr::Name(t, component))
    }
}

fn is_name(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    fn custom(text: &str) -> Custom {
        Custom::parse(&Properties::parse(text, &Macros::standard()))
    }

    #[test]
    fn expressions_follow_c_precedence_and_file_order() {
        let mut c = custom(
            "variable.float.a = 1 + 2 * 3 - 4 % 3\n\
             uniform.float.b = a / 2\n\
             uniform.bool.c = a > 5 && !(b < 1) || false\n\
             uniform.int.d = -7 / 2\n\
             uniform.vec3.e = vec3(a, cameraPosition.y, 2) * 2\n\
             uniform.float.f = if(a < 0, 1, a < 7, 2, 3) + clamp(fmod(-1, 4), 0, 2.5) + min(4, 2, 3)\n\
             uniform.float.g = gbufferModelView.0.1 + frac(sunAngle - 0.25) + pi * 0\n\
             uniform.float.h = unknownThing + in(biome, 3, BIOME_DESERT)\n\
             uniform.int.i = biome == BIOME_PALE_GARDEN || biome == BIOME_OLD\n\
             uniform.int.j = CAT_OCEAN + PPT_RAIN\n",
        );
        c.set_biomes(&["minecraft:beach", "minecraft:desert"]);
        assert!(c.notes.is_empty(), "{:?}", c.notes);
        let names: Vec<_> = c.uniforms().map(|u| u.0).collect();
        assert_eq!(names, ["b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let builtin = |n: &str| match n {
            "cameraPosition" => Some(Val::vector(&[1.0, 64.0, 2.0])),
            "sunAngle" => Some(Val::scalar(0.125)),
            "biome" => Some(Val::scalar(1.0)),
            "gbufferModelView" => {
                let mut m = Val::scalar(0.0);
                m.v[1] = 9.0; // column 0, row 1
                m.n = 16;
                Some(m)
            }
            _ => None,
        };
        let v = c.eval(0.016, &builtin);
        let x: Vec<f64> = v.iter().map(|v| v.v[0]).collect();
        assert_eq!(
            x,
            [
                3.0,
                1.0,
                -3.0,
                12.0,
                2.0 + 2.5 + 2.0,
                9.0 + 0.875,
                1.0,
                0.0,
                12.0
            ]
        );
        assert_eq!(&v[3].v[..3], &[12.0, 128.0, 4.0]);
        assert_eq!(
            c.notes,
            [
                "`unknownThing` is unknown; it reads 0",
                "`BIOME_PALE_GARDEN` names no biome; it reads -1",
                "`BIOME_OLD` names no biome; it reads -1"
            ]
        );
    }

    #[test]
    fn smooth_halves_the_distance_per_fade_time() {
        let mut c = custom("uniform.float.s = smooth(1, target, 2, 0.5)\n");
        let mut target = 0.0;
        let mut run = |t: f64, dt: f64| {
            target = t;
            c.eval(dt, &|n: &str| (n == "target").then(|| Val::scalar(target)))[0].v[0]
        };
        assert_eq!(run(0.0, 0.0), 0.0);
        assert!((run(1.0, 2.0) - 0.5).abs() < 1e-9, "rising: fadeIn");
        assert!((run(0.0, 0.5) - 0.25).abs() < 1e-9, "falling: fadeOut");
    }

    /// After a restart `smooth` starts from its next value, as on the first frame, instead of
    /// fading from the old one (a 1000-second fade from a dry run's zero).
    #[test]
    fn smooth_starts_again_from_its_next_value_after_a_restart() {
        let mut c = custom("uniform.float.s = smooth(target, 1000, 1000)\n");
        let eval = |c: &mut Custom, t: f64| {
            c.eval(0.05, &|n: &str| (n == "target").then(|| Val::scalar(t)))[0].v[0]
        };
        assert_eq!(eval(&mut c, 0.0), 0.0);
        assert!(eval(&mut c, 0.6) < 0.01, "the fade holds the old value");
        c.restart_smoothing();
        assert_eq!(eval(&mut c, 0.6), 0.6);
    }

    #[test]
    fn bad_expressions_are_reported() {
        let c = custom("uniform.float.a = (1 + \nuniform.mat4.m = 1\nuniform.float.ok = 2\n");
        assert_eq!(c.uniforms().count(), 1);
        assert_eq!(c.notes.len(), 2, "{:?}", c.notes);
    }
}
