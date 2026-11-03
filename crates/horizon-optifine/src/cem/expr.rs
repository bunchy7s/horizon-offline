//! CEM animation expressions (`"head.rx": "sin(limb_swing * 0.6662) * limb_speed"`), compiled
//! once when the packs load into a flat stack program, then evaluated per entity per frame
//! without allocating or touching strings.
//!
//! Variables are slots: the caller resolves each name (`limb_swing`, `head.rx`, `var.x`...) to
//! an index into its own `f32` array when compiling. Booleans are 1 and 0, and any value but 0
//! is true, NaN included. `if`, `&&` and `||` evaluate only the branch they take.

use super::curves::{self, Curve, Ease};
use std::cell::{Cell, RefCell};
use std::fmt;

/// The value stack on the program's own frame; deeper programs (long `keyframe` tracks) use a
/// buffer kept per thread.
const STACK: usize = 64;
/// Deepest nesting of brackets, unary operators and calls, so that no pack can overflow the
/// thread's stack while it parses.
const NESTING: usize = 64;
/// Most nodes an expression may parse to, which bounds the folding, the code and its stack.
const NODES: usize = 1024;

/// Why an expression doesn't compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// What is wrong, in English.
    pub message: String,
    /// Byte offset in the expression where it went wrong.
    pub offset: usize,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} at {}", self.message, self.offset)
    }
}
impl std::error::Error for Error {}

fn err<T>(message: impl Into<String>, offset: usize) -> Result<T, Error> {
    Err(Error {
        message: message.into(),
        offset,
    })
}

/// A compiled expression.
#[derive(Clone, Debug)]
pub struct Expr {
    code: Vec<Op>,
    /// The most values on its stack at once.
    depth: usize,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Const(f32),
    Slot(u32),
    Neg,
    Bin(Bin),
    Call(Func, u16),
    /// Pops; jumps when the value is 0 (not when it is NaN).
    JumpIfZero(u32),
    Jump(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}
impl Bin {
    fn apply(self, a: f32, b: f32) -> f32 {
        let t = |c: bool| if c { 1.0 } else { 0.0 };
        match self {
            Bin::Add => a + b,
            Bin::Sub => a - b,
            Bin::Mul => a * b,
            Bin::Div => a / b,
            Bin::Rem => a % b,
            Bin::Lt => t(a < b),
            Bin::Le => t(a <= b),
            Bin::Gt => t(a > b),
            Bin::Ge => t(a >= b),
            Bin::Eq => t(a == b),
            Bin::Ne => t(a != b),
        }
    }
}

macro_rules! funcs {
    ($($name:literal => $f:ident ($min:literal, $max:expr)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Func {
            $($f,)*
            /// `ease{in,out,inout}{quad,cubic...}(k, x, y)`
            Ease(Curve, Ease),
        }
        impl Func {
            fn named(name: &str) -> Option<(Func, usize, usize)> {
                if let Some(rest) = name.strip_prefix("ease") {
                    let (ease, curve) = if let Some(c) = rest.strip_prefix("inout") {
                        (Ease::InOut, c)
                    } else if let Some(c) = rest.strip_prefix("in") {
                        (Ease::In, c)
                    } else {
                        (Ease::Out, rest.strip_prefix("out")?)
                    };
                    return Some((Func::Ease(Curve::named(curve)?, ease), 3, 3));
                }
                Some(match name { $($name => (Func::$f, $min, $max),)* _ => return None })
            }
        }
    };
}
const MANY: usize = usize::MAX;
funcs! {
    "sin" => Sin(1, 1), "cos" => Cos(1, 1), "asin" => Asin(1, 1), "acos" => Acos(1, 1),
    "tan" => Tan(1, 1), "atan" => Atan(1, 1), "atan2" => Atan2(2, 2), "torad" => Torad(1, 1),
    "todeg" => Todeg(1, 1), "min" => Min(1, MANY), "max" => Max(1, MANY),
    "clamp" => Clamp(3, 3), "abs" => Abs(1, 1), "floor" => Floor(1, 1), "ceil" => Ceil(1, 1),
    "exp" => Exp(1, 1), "frac" => Frac(1, 1), "log" => Log(1, 1), "pow" => Pow(2, 2),
    "random" => Random(0, 1), "randomb" => Randomb(0, 1), "round" => Round(1, 1),
    "signum" => Signum(1, 1), "sqrt" => Sqrt(1, 1), "fmod" => Fmod(2, 2), "lerp" => Lerp(3, 3),
    "print" => Print(3, 3), "printb" => Printb(3, 3), "between" => Between(3, 3),
    "equals" => Equals(3, 3), "in" => In(2, MANY), "wrapdeg" => Wrapdeg(1, 1),
    "wraprad" => Wraprad(1, 1), "degdiff" => Degdiff(2, 2), "raddiff" => Raddiff(2, 2),
    "catch" => Catch(2, 3), "keyframe" => Keyframe(2, MANY), "keyframeloop" => Keyframeloop(2, MANY),
    "catmullrom" => Catmullrom(5, 5), "hermite" => Hermite(5, 5), "cubicbezier" => Cubicbezier(5, 5),
    "quadbezier" => Quadbezier(4, 4),
}

const PI: f32 = std::f32::consts::PI;

fn wrap(x: f32, turn: f32) -> f32 {
    let r = x % turn;
    if r >= turn / 2.0 {
        r - turn
    } else if r < -turn / 2.0 {
        r + turn
    } else {
        r
    }
}

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

thread_local! {
    /// The unseeded `random()` sequence: every thread starts somewhere else, every launch too.
    static RANDOM: Cell<u64> = Cell::new(random_start());
}

fn random_start() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static THREADS: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    now ^ THREADS.fetch_add(1, Ordering::Relaxed).wrapping_mul(GOLDEN)
}

fn random(seed: Option<f32>) -> f32 {
    let mut x = match seed {
        Some(s) => u64::from(s.to_bits()).wrapping_mul(GOLDEN) ^ 0xD1B5_4A32_D192_ED03,
        None => RANDOM.with(|s| {
            let v = s.get().wrapping_add(GOLDEN);
            s.set(v);
            v
        }),
    };
    // splitmix64's finaliser.
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

impl Func {
    /// Whether two calls with the same arguments may give different results.
    fn varies(self, argc: usize) -> bool {
        matches!(self, Func::Random | Func::Randomb) && argc == 0
    }

    fn apply(self, a: &[f32]) -> f32 {
        use Func::*;
        let t = |c: bool| if c { 1.0 } else { 0.0 };
        match self {
            Sin => a[0].sin(),
            Cos => a[0].cos(),
            Asin => a[0].asin(),
            Acos => a[0].acos(),
            Tan => a[0].tan(),
            Atan => a[0].atan(),
            Atan2 => a[0].atan2(a[1]),
            Torad => a[0].to_radians(),
            Todeg => a[0].to_degrees(),
            // Java's `Math.min` and `max`: NaN if any argument is.
            Min | Max if a.iter().any(|v| v.is_nan()) => f32::NAN,
            Min => a.iter().copied().fold(f32::INFINITY, f32::min),
            Max => a.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            // Java's `x < lo ? lo : Math.min(x, hi)`: a NaN `x` stays NaN.
            Clamp if a[0] < a[1] => a[1],
            Clamp if a[0].is_nan() || a[2].is_nan() => f32::NAN,
            Clamp => a[0].min(a[2]),
            Abs => a[0].abs(),
            Floor => a[0].floor(),
            Ceil => a[0].ceil(),
            Exp => a[0].exp(),
            Frac => a[0] - a[0].floor(),
            Log => a[0].ln(),
            // Java's `Math.pow`, where C's differs: a NaN exponent is NaN, and so is 1 or -1 to
            // an infinite one. Any `x` to 0 is 1, NaN too, in both.
            Pow if a[1].is_nan() || (a[0].abs() == 1.0 && a[1].is_infinite()) => f32::NAN,
            Pow => a[0].powf(a[1]),
            Random => random(a.first().copied()),
            Randomb => t(random(a.first().copied()) >= 0.5),
            // Java's `Math.round(float)`, an `int`: halves go up, NaN is 0 and the rest saturates
            // at the `int`'s ends. `x - floor(x)` is exact, `x + 0.5` isn't.
            Round => {
                let f = a[0].floor();
                (if a[0] - f >= 0.5 { f + 1.0 } else { f }) as i32 as f32
            }
            Signum if a[0] == 0.0 || a[0].is_nan() => a[0],
            Signum => a[0].signum(),
            Sqrt => a[0].sqrt(),
            Fmod => {
                let r = a[0] % a[1];
                if r != 0.0 && (r < 0.0) != (a[1] < 0.0) {
                    r + a[1]
                } else {
                    r
                }
            }
            Lerp => curves::lerp(a[0], a[1], a[2]),
            // Printing to the log is a debugging aid: only the value matters here.
            Print | Printb => a[2],
            Between => t(a[1] <= a[0] && a[0] <= a[2]),
            Equals => t((a[0] - a[1]).abs() <= a[2]),
            In => t(a[1..].contains(&a[0])),
            Wrapdeg => wrap(a[0], 360.0),
            Wraprad => wrap(a[0], 2.0 * PI),
            Degdiff => wrap(a[1] - a[0], 360.0),
            Raddiff => wrap(a[1] - a[0], 2.0 * PI),
            // EMF: x unless it is NaN; infinities pass.
            Catch if a[0].is_nan() => a[1],
            Catch => a[0],
            Keyframe => curves::keyframe(a[0], &a[1..], false),
            Keyframeloop => curves::keyframe(a[0], &a[1..], true),
            Catmullrom => curves::catmullrom(a[0], a[1], a[2], a[3], a[4]),
            Hermite => curves::hermite(a[0], a[1], a[2], a[3], a[4]),
            Cubicbezier => curves::cubic_bezier(a[0], a[1], a[2], a[3], a[4]),
            Quadbezier => curves::quad_bezier(a[0], a[1], a[2], a[3]),
            // `lerp` along the curve.
            Ease(curve, ease) => curves::lerp(curve.at(ease, a[0]), a[1], a[2]),
        }
    }
}

/// The parsed form, folded before it becomes code. `!`, `&&`, `||` and `ifb` are lowered to
/// comparisons with 0 and `if`s.
enum Node {
    Num(f32),
    Slot(u32),
    Neg(Box<Node>),
    Bin(Bin, Box<Node>, Box<Node>),
    /// cond, value, [cond, value...], else
    If(Vec<Node>),
    Call(Func, Vec<Node>),
}

fn bin(op: Bin, a: Node, b: Node) -> Node {
    Node::Bin(op, Box::new(a), Box::new(b))
}

/// `a` as a boolean, 1 or 0.
fn truth(a: Node) -> Node {
    bin(Bin::Ne, a, Node::Num(0.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok<'a> {
    Num(f32),
    Name(&'a str),
    Sym(&'a str),
    End,
}

struct Parser<'a, 'r> {
    src: &'a str,
    pos: usize,
    tok: Tok<'a>,
    at: usize,
    /// Open brackets, unary operators and calls around the current token.
    nesting: usize,
    nodes: usize,
    resolve: &'r mut dyn FnMut(&str) -> Option<u32>,
}

impl<'a> Parser<'a, '_> {
    fn next(&mut self) -> Result<(), Error> {
        let b = self.src.as_bytes();
        while self.pos < b.len() && b[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        self.at = self.pos;
        let Some(&c) = b.get(self.pos) else {
            self.tok = Tok::End;
            return Ok(());
        };
        let start = self.pos;
        let word = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':');
        if c.is_ascii_digit() || c == b'.' && b.get(start + 1).is_some_and(u8::is_ascii_digit) {
            while self.pos < b.len() && (b[self.pos].is_ascii_digit() || b[self.pos] == b'.') {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            self.tok = Tok::Num(match text.parse() {
                Ok(v) => v,
                Err(_) => return err(format!("bad number {text}"), start),
            });
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while self.pos < b.len() && word(b[self.pos]) {
                self.pos += 1;
            }
            self.tok = Tok::Name(&self.src[start..self.pos]);
        } else {
            let two = self.src.get(start..start + 2);
            let len = match two {
                Some("&&" | "||" | "<=" | ">=" | "==" | "!=") => 2,
                _ if b"+-*/%!<>(),".contains(&c) => 1,
                _ => return err(format!("unexpected {:?}", c as char), start),
            };
            self.pos += len;
            self.tok = Tok::Sym(&self.src[start..self.pos]);
        }
        Ok(())
    }

    fn eat(&mut self, sym: &str) -> Result<bool, Error> {
        if self.tok == Tok::Sym(sym) {
            self.next()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn expect(&mut self, sym: &str) -> Result<(), Error> {
        if !self.eat(sym)? {
            return err(format!("expected {sym}"), self.at);
        }
        Ok(())
    }

    /// One more level of nesting, opened at `at`.
    fn enter(&mut self, at: usize) -> Result<(), Error> {
        self.nesting += 1;
        if self.nesting > NESTING {
            return err("expression too deep", at);
        }
        Ok(())
    }

    /// `n` more nodes.
    fn grow(&mut self, n: usize) -> Result<(), Error> {
        self.nodes += n;
        if self.nodes > NODES {
            return err("expression too long", self.at);
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Node, Error> {
        let mut a = self.and()?;
        while self.eat("||")? {
            let b = self.and()?;
            self.grow(4)?;
            a = Node::If(vec![a, Node::Num(1.0), truth(b)]);
        }
        Ok(a)
    }

    fn and(&mut self) -> Result<Node, Error> {
        let mut a = self.binary(0)?;
        while self.eat("&&")? {
            let b = self.binary(0)?;
            self.grow(4)?;
            a = Node::If(vec![a, truth(b), Node::Num(0.0)]);
        }
        Ok(a)
    }

    /// Equality, then comparisons, then sums, then products.
    fn binary(&mut self, level: usize) -> Result<Node, Error> {
        const LEVELS: [&[(&str, Bin)]; 4] = [
            &[("==", Bin::Eq), ("!=", Bin::Ne)],
            &[
                ("<=", Bin::Le),
                (">=", Bin::Ge),
                ("<", Bin::Lt),
                (">", Bin::Gt),
            ],
            &[("+", Bin::Add), ("-", Bin::Sub)],
            &[("*", Bin::Mul), ("/", Bin::Div), ("%", Bin::Rem)],
        ];
        let Some(ops) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut a = self.binary(level + 1)?;
        'more: loop {
            for (sym, op) in ops.iter() {
                if self.eat(sym)? {
                    let b = self.binary(level + 1)?;
                    self.grow(1)?;
                    a = bin(*op, a, b);
                    continue 'more;
                }
            }
            return Ok(a);
        }
    }

    fn unary(&mut self) -> Result<Node, Error> {
        let at = self.at;
        if let Tok::Sym(sym @ ("-" | "+" | "!")) = self.tok {
            self.next()?;
            self.enter(at)?;
            let a = self.unary()?;
            self.nesting -= 1;
            self.grow(2)?;
            return Ok(match sym {
                "-" => Node::Neg(Box::new(a)),
                "!" => bin(Bin::Eq, a, Node::Num(0.0)),
                _ => a,
            });
        }
        self.grow(1)?;
        match self.tok {
            Tok::Num(v) => {
                self.next()?;
                Ok(Node::Num(v))
            }
            Tok::Sym("(") => {
                self.next()?;
                self.enter(at)?;
                let e = self.or()?;
                self.nesting -= 1;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Name(name) => {
                self.next()?;
                if self.tok == Tok::Sym("(") {
                    return self.call(name, at);
                }
                Ok(match name {
                    "pi" => Node::Num(PI),
                    "e" => Node::Num(std::f32::consts::E),
                    "nan" => Node::Num(f32::NAN),
                    "true" => Node::Num(1.0),
                    "false" => Node::Num(0.0),
                    _ => match (self.resolve)(name) {
                        Some(slot) => Node::Slot(slot),
                        None => return err(format!("unknown variable {name}"), at),
                    },
                })
            }
            Tok::End => err("unexpected end", at),
            Tok::Sym(s) => err(format!("unexpected {s}"), at),
        }
    }

    /// Skips the raw text of a call, which isn't an expression, to its closing bracket.
    fn skip_call(&mut self) -> Result<(), Error> {
        let b = self.src.as_bytes();
        let mut open = 1;
        while open > 0 {
            match b.get(self.pos) {
                None => return err("expected )", self.pos),
                Some(b'(') => open += 1,
                Some(b')') => open -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        self.next()
    }

    fn call(&mut self, name: &str, at: usize) -> Result<Node, Error> {
        if name == "nbt" {
            // EMF's `nbt(key, test)` needs the entity's NBT, which the client model doesn't
            // have; its test (`exists:false`...) isn't an expression either.
            self.skip_call()?;
            return Ok(Node::Num(0.0));
        }
        self.enter(self.at)?;
        self.expect("(")?;
        let mut args = Vec::new();
        if !self.eat(")")? {
            loop {
                args.push(self.or()?);
                if self.eat(")")? {
                    break;
                }
                self.expect(",")?;
            }
        }
        self.nesting -= 1;
        if name == "if" || name == "ifb" {
            if args.len() < 3 || args.len() % 2 == 0 {
                return err(format!("{name} takes cond, value pairs and an else"), at);
            }
            if name == "ifb" {
                self.grow(2)?;
                return Ok(truth(Node::If(args)));
            }
            return Ok(Node::If(args));
        }
        let Some((f, min, max)) = Func::named(name) else {
            return err(format!("unknown function {name}"), at);
        };
        if args.len() < min || args.len() > max.min(u16::MAX as usize) {
            return err(format!("wrong number of arguments to {name}"), at);
        }
        Ok(Node::Call(f, args))
    }
}

/// Folds the constant parts of `n`.
fn fold(n: Node) -> Node {
    let num = |n: &Node| match n {
        Node::Num(v) => Some(*v),
        _ => None,
    };
    match n {
        Node::Neg(a) => match fold(*a) {
            Node::Num(v) => Node::Num(-v),
            a => Node::Neg(Box::new(a)),
        },
        Node::Bin(op, a, b) => match (fold(*a), fold(*b)) {
            (Node::Num(x), Node::Num(y)) => Node::Num(op.apply(x, y)),
            (a, b) => bin(op, a, b),
        },
        Node::If(args) => {
            let mut out = Vec::new();
            let mut args = args.into_iter();
            while args.len() > 1 {
                let (c, v) = (fold(args.next().unwrap()), args.next().unwrap());
                match num(&c) {
                    // A false branch is dropped, a true one (NaN too) ends the chain.
                    Some(0.0) => continue,
                    Some(_) if out.is_empty() => return fold(v),
                    Some(_) => {
                        out.push(fold(v));
                        return Node::If(out);
                    }
                    None => out.extend([c, fold(v)]),
                }
            }
            let otherwise = fold(args.next().unwrap());
            if out.is_empty() {
                return otherwise;
            }
            out.push(otherwise);
            Node::If(out)
        }
        Node::Call(f, args) => {
            let args: Vec<Node> = args.into_iter().map(fold).collect();
            match args.iter().map(num).collect::<Option<Vec<f32>>>() {
                Some(values) if !f.varies(values.len()) => Node::Num(f.apply(&values)),
                _ => Node::Call(f, args),
            }
        }
        n => n,
    }
}

struct Emit {
    code: Vec<Op>,
    depth: usize,
    max: usize,
}
impl Emit {
    fn push(&mut self, op: Op, change: isize) {
        self.code.push(op);
        self.depth = self
            .depth
            .checked_add_signed(change)
            .expect("stack underflow");
        self.max = self.max.max(self.depth);
    }
    fn here(&self) -> u32 {
        self.code.len() as u32
    }
    fn patch(&mut self, at: u32) {
        let to = self.here();
        match &mut self.code[at as usize] {
            Op::JumpIfZero(t) | Op::Jump(t) => *t = to,
            _ => unreachable!(),
        }
    }
    fn node(&mut self, n: &Node) {
        match n {
            Node::Num(v) => self.push(Op::Const(*v), 1),
            Node::Slot(s) => self.push(Op::Slot(*s), 1),
            Node::Neg(a) => {
                self.node(a);
                self.push(Op::Neg, 0);
            }
            Node::Bin(op, a, b) => {
                self.node(a);
                self.node(b);
                self.push(Op::Bin(*op), -1);
            }
            Node::If(args) => {
                let mut ends = Vec::new();
                for pair in args.chunks_exact(2) {
                    self.node(&pair[0]);
                    let next = self.here();
                    self.push(Op::JumpIfZero(0), -1);
                    self.node(&pair[1]);
                    ends.push(self.here());
                    self.push(Op::Jump(0), -1);
                    self.patch(next);
                }
                self.node(args.last().unwrap());
                for end in ends {
                    self.patch(end);
                }
            }
            Node::Call(f, args) => {
                for a in args {
                    self.node(a);
                }
                self.push(Op::Call(*f, args.len() as u16), 1 - args.len() as isize);
            }
        }
    }
}

/// Compiles `src`. `resolve` gives the slot of a variable name, None for one that doesn't
/// exist.
pub fn compile(src: &str, resolve: &mut dyn FnMut(&str) -> Option<u32>) -> Result<Expr, Error> {
    let mut p = Parser {
        src,
        pos: 0,
        tok: Tok::End,
        at: 0,
        nesting: 0,
        nodes: 0,
        resolve,
    };
    p.next()?;
    let tree = p.or()?;
    if p.tok != Tok::End {
        return err("unexpected text after the expression", p.at);
    }
    let tree = fold(tree);
    let mut emit = Emit {
        code: Vec::new(),
        depth: 0,
        max: 0,
    };
    emit.node(&tree);
    Ok(Expr {
        code: emit.code,
        depth: emit.max,
    })
}

thread_local! {
    /// The stack of programs too deep for `STACK`, grown once and kept.
    static DEEP: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

impl Expr {
    /// The value with the variables in `slots`.
    pub fn eval(&self, slots: &[f32]) -> f32 {
        if self.depth <= STACK {
            return self.run(slots, &mut [0.0; STACK]);
        }
        DEEP.with_borrow_mut(|stack| {
            if stack.len() < self.depth {
                stack.resize(self.depth, 0.0);
            }
            self.run(slots, stack)
        })
    }

    #[inline(always)]
    fn run(&self, slots: &[f32], stack: &mut [f32]) -> f32 {
        let mut sp = 0;
        let mut pc = 0;
        while let Some(op) = self.code.get(pc) {
            pc += 1;
            match *op {
                Op::Const(v) => {
                    stack[sp] = v;
                    sp += 1;
                }
                Op::Slot(s) => {
                    stack[sp] = slots.get(s as usize).copied().unwrap_or(0.0);
                    sp += 1;
                }
                Op::Neg => stack[sp - 1] = -stack[sp - 1],
                Op::Bin(op) => {
                    sp -= 1;
                    stack[sp - 1] = op.apply(stack[sp - 1], stack[sp]);
                }
                Op::Call(f, n) => {
                    let n = n as usize;
                    let v = f.apply(&stack[sp - n..sp]);
                    sp -= n;
                    stack[sp] = v;
                    sp += 1;
                }
                Op::JumpIfZero(to) => {
                    sp -= 1;
                    if stack[sp] == 0.0 {
                        pc = to as usize;
                    }
                }
                Op::Jump(to) => pc = to as usize,
            }
        }
        stack[0]
    }

    /// The value, when it doesn't depend on any variable or on chance.
    #[cfg(test)]
    fn constant(&self) -> Option<f32> {
        match self.code.as_slice() {
            [Op::Const(v)] => Some(*v),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slots: `a` 0, `b` 1, `var.n` 2, `head.rx` 3.
    const NAMES: [&str; 4] = ["a", "b", "var.n", "head.rx"];
    fn names(n: &str) -> Option<u32> {
        NAMES.iter().position(|x| *x == n).map(|i| i as u32)
    }
    fn expr(src: &str) -> Expr {
        compile(src, &mut names).unwrap_or_else(|e| panic!("{src}: {e}"))
    }
    fn eval(src: &str, slots: &[f32]) -> f32 {
        expr(src).eval(slots)
    }
    fn value(src: &str) -> f32 {
        eval(src, &[])
    }
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn precedence_and_booleans() {
        assert_eq!(value("1 + 2 * 3"), 7.0);
        assert_eq!(value("(1 + 2) * 3"), 9.0);
        assert_eq!(value("-2 * 3 + 10 % 4"), -4.0);
        assert_eq!(value("1 < 2 == 1"), 1.0);
        assert_eq!(value("1 + 1 == 2 && !(3 > 4) || false"), 1.0);
        assert_eq!(value("2 && 0.5"), 1.0);
        assert_eq!(value("1 - -1"), 2.0);
        assert_eq!(value("+2 * +(1 + 2) - +-1"), 7.0);
        assert_eq!(eval("a * 2 + b", &[3.0, 1.0]), 7.0);
        assert_eq!(eval("head.rx + var.n", &[0.0, 0.0, 2.0, 0.5]), 2.5);
        // The same through slots, so nothing folds.
        let neg_not = expr("-a + !a");
        assert_eq!([0.0, 3.0].map(|a| neg_not.eval(&[a])), [1.0, -3.0]);
        let and = expr("1 && a");
        assert_eq!([5.0, 0.0].map(|a| and.eval(&[a])), [1.0, 0.0]);
        assert_eq!(eval("a || b", &[0.0, 7.0]), 1.0);
        // NaN is true.
        assert_eq!(eval("a && 1", &[f32::NAN]), 1.0);
        assert_eq!(eval("!a", &[f32::NAN]), 0.0);
        assert_eq!(eval("if(a, 2, 3)", &[f32::NAN]), 2.0);
        assert_eq!(value("if(nan, 2, 3)"), 2.0);
    }

    #[test]
    fn branches_not_taken_are_not_evaluated() {
        // A `random()` that runs advances the thread's sequence: replay it from one start.
        RANDOM.with(|s| s.set(12345));
        let first = value("random()");
        RANDOM.with(|s| s.set(12345));
        for (src, slots, want) in [
            ("if(a, random(), b)", [0.0, 5.0], 5.0),
            ("if(a > 1, random(), b > 1, random(), 4)", [0.0, 0.0], 4.0),
            ("a && random()", [0.0, 0.0], 0.0),
            ("b || random()", [0.0, 1.0], 1.0),
        ] {
            assert_eq!(expr(src).eval(&slots), want, "{src}");
        }
        assert_eq!(value("random()"), first);
        // Each condition in order, then the else.
        let chain = expr("if(a > 2, 10, a > 1, 20, 30)");
        assert_eq!(
            [3.0, 1.5, 0.0].map(|a| chain.eval(&[a])),
            [10.0, 20.0, 30.0]
        );
    }

    #[test]
    fn unseeded_random_differs_between_threads() {
        let draw = || std::thread::spawn(|| value("random()")).join().unwrap();
        assert_ne!(draw(), draw());
    }

    #[test]
    fn functions() {
        let pi = std::f32::consts::PI;
        assert_eq!(value("min(4, 2, 8, 3)"), 2.0);
        assert_eq!(value("max(4, 2, 8, 3)"), 8.0);
        assert!(value("min(nan, 1)").is_nan());
        assert!(eval("max(1, a)", &[f32::NAN]).is_nan());
        assert_eq!(value("clamp(5, 0, 2)"), 2.0);
        assert_eq!(value("clamp(-1, 0, 2)"), 0.0);
        assert_eq!(value("clamp(1 / 0, 0, 2)"), 2.0);
        assert_eq!(value("clamp(-1 / 0, 0, 2)"), 0.0);
        assert!(value("clamp(nan, 0, 2)").is_nan());
        assert!(eval("clamp(a, 0, 2)", &[f32::NAN]).is_nan());
        assert!(value("clamp(1, 0, nan)").is_nan());
        // Java's `Math.pow`, not C's.
        assert!(value("pow(1, nan)").is_nan());
        assert!(eval("pow(a, b)", &[1.0, f32::NAN]).is_nan());
        assert!(value("pow(1, 1 / 0)").is_nan());
        assert!(value("pow(-1, -1 / 0)").is_nan());
        assert!(eval("pow(a, b)", &[-1.0, f32::INFINITY]).is_nan());
        assert_eq!(value("pow(nan, 0)"), 1.0);
        assert_eq!(eval("pow(a, 0)", &[f32::NAN]), 1.0);
        assert_eq!(value("pow(1 / 0, 0)"), 1.0);
        assert_eq!(value("-7 % 3"), -1.0);
        assert_eq!(value("fmod(-7, 3)"), 2.0);
        assert_eq!(value("frac(-1.25)"), 0.75);
        assert_eq!(value("round(2.5) + round(-2.5)"), 1.0);
        assert_eq!(value("round(-2.5)"), -2.0);
        assert_eq!(value("round(0.49999997)"), 0.0);
        assert_eq!(value("round(16777215)"), 16_777_215.0);
        // An `int`: NaN is 0, the rest stops at its ends.
        assert_eq!(value("round(nan)"), 0.0);
        assert_eq!(eval("round(a)", &[f32::NAN]), 0.0);
        assert_eq!(value("round(1 / 0)"), 2_147_483_648.0);
        assert_eq!(value("round(-1 / 0)"), -2_147_483_648.0);
        assert_eq!(value("round(10000000000)"), 2_147_483_648.0);
        assert_eq!(eval("round(a)", &[-1e10]), -2_147_483_648.0);
        assert_eq!(value("signum(-3) + signum(0)"), -1.0);
        assert!(close(value("torad(180)"), pi));
        assert!(close(value("todeg(pi / 2)"), 90.0));
        assert_eq!(value("lerp(0.25, 4, 8)"), 5.0);
        assert_eq!(value("between(2, 1, 3) + between(4, 1, 3)"), 1.0);
        assert_eq!(value("equals(1, 1.05, 0.1) + in(3, 1, 2, 3)"), 2.0);
        assert_eq!(value("print(1, 20, 6)"), 6.0);
        assert_eq!(value("random(4)"), value("random(4)"));
        assert!((0.0..1.0).contains(&value("random(4)")));
        assert_eq!(value("catch(0 / 0, 3)"), 3.0);
        assert_eq!(value("catch(1 / 0, 3)"), f32::INFINITY);
        assert_eq!(value("catch(2, 3, 1)"), 2.0);
        // `ifb` is a boolean.
        let ifb = expr("ifb(a, 5, 0)");
        assert_eq!([1.0, 0.0].map(|a| ifb.eval(&[a])), [1.0, 0.0]);
        // EMF's form; the client has no NBT, so never a match.
        assert_eq!(value("nbt(SaddleItem, exists:false)"), 0.0);
        assert_eq!(eval("a + nbt(Tags, raw:(x)) + 1", &[2.0]), 3.0);
        // The runtime path of the maths.
        let atan2 = expr("atan2(a, 1)");
        assert!(close(atan2.eval(&[1.0]), pi / 4.0));
        assert!(close(
            eval("pow(a, 3) + sqrt(16) + ceil(-1.5) + log(e)", &[2.0]),
            12.0
        ));
        // EMF's documented examples.
        assert_eq!(value("wrapdeg(370)"), 10.0);
        assert_eq!(value("wrapdeg(350)"), -10.0);
        assert_eq!(value("degdiff(10, 350)"), -20.0);
        assert_eq!(value("degdiff(10, 370)"), 0.0);
    }

    #[test]
    fn keyframes_and_curves() {
        let at = |k: f32, f: &str| eval(&format!("{f}(a, 5, 10, 20)"), &[k]);
        assert_eq!(at(0.0, "keyframe"), 5.0);
        assert_eq!(at(1.0, "keyframe"), 10.0);
        assert_eq!(at(1.5, "keyframe"), 15.0);
        assert_eq!(at(7.0, "keyframe"), 20.0);
        assert_eq!(at(-1.5, "keyframe"), 15.0);
        assert_eq!(at(f32::INFINITY, "keyframe"), 20.0);
        assert!(at(f32::NAN, "keyframe").is_nan());
        assert!(at(f32::INFINITY, "keyframeloop").is_nan());
        assert_eq!(value("keyframe(1 / 0, 5, 10)"), 10.0);
        // The loop goes from the last frame back to the first.
        assert_eq!(at(2.5, "keyframeloop"), 12.5);
        assert_eq!(at(3.5, "keyframeloop"), 7.5);
        // A long track: frames 0 to 299, deeper than the fixed stack, more arguments than a byte
        // counts.
        let frames: Vec<String> = (0..300).map(|i| i.to_string()).collect();
        let track = expr(&format!("keyframe(a, {})", frames.join(", ")));
        assert_eq!(track.eval(&[298.5]), 298.5);
        assert_eq!(track.eval(&[3.0]), 3.0);
        assert_eq!(track.eval(&[1000.0]), 299.0);
        // Ends of every curve are its two values.
        for f in [
            "easeinoutexpo",
            "easeinquad",
            "easeoutbounce",
            "easeinoutelastic",
            "easeinoutback",
            "easeincirc",
            "easeoutsine",
            "easeinoutquint",
            "easeoutelastic",
            "easeinbounce",
        ] {
            assert!(close(eval(&format!("{f}(a, 2, 6)"), &[0.0]), 2.0), "{f}");
            assert!(close(eval(&format!("{f}(a, 2, 6)"), &[1.0]), 6.0), "{f}");
        }
        assert_eq!(value("easeinquad(0.5, 0, 8)"), 2.0);
        assert_eq!(value("easeoutquad(0.5, 0, 8)"), 6.0);
        // Mid-curve, from 0 to 1.
        for (f, t, want) in [
            ("easeoutbounce", 0.5, 0.765625),
            ("easeoutelastic", 0.5, 1.015625),
            ("easeinoutexpo", 0.25, 0.015625),
            ("easeinoutcubic", 0.25, 0.0625),
            ("easeinback", 0.5, -0.0876975),
        ] {
            let got = eval(&format!("{f}(a, 0, 1)"), &[t]);
            assert!(close(got, want), "{f}({t}) = {got}, not {want}");
        }
        assert_eq!(value("quadbezier(0.5, 0, 4, 2)"), 2.0);
        assert_eq!(value("cubicbezier(0.5, 0, 8, 0, 8)"), 4.0);
        assert_eq!(value("catmullrom(0, 0, 1, 2, 3)"), 1.0);
        assert_eq!(value("catmullrom(1, 0, 1, 2, 3)"), 2.0);
        assert_eq!(value("hermite(1, 3, 7, 0, 0)"), 7.0);
    }

    #[test]
    fn constants_fold() {
        let pi = std::f32::consts::PI;
        assert!(close(expr("torad(180)").constant().unwrap(), pi));
        assert_eq!(expr("if(true, 2, a)").constant(), Some(2.0));
        assert_eq!(expr("if(false, a, 1 > 0, 3, b)").constant(), Some(3.0));
        assert_eq!(expr("0 && a").constant(), Some(0.0));
        assert_eq!(expr("1 || a").constant(), Some(1.0));
        assert_eq!(expr("random()").constant(), None);
        assert_eq!(expr("a + 1").constant(), None);
        // A false branch goes; the variable ones stay.
        let partly = expr("if(false, a, b > 1, b, 4)");
        assert!(!partly.code.iter().any(|op| matches!(op, Op::Slot(0))));
        assert_eq!(partly.eval(&[9.0, 2.0]), 2.0);
        assert_eq!(partly.eval(&[9.0, 0.0]), 4.0);
    }

    #[test]
    fn errors_say_where() {
        let e = |src: &str| compile(src, &mut names).unwrap_err();
        assert_eq!(e("a + nope").offset, 4);
        assert!(e("a + nope").message.contains("nope"));
        assert_eq!(e("sinh(a)").offset, 0);
        assert!(e("clamp(1, 2)").message.contains("arguments"));
        assert!(e("if(a, b)").message.contains("if"));
        assert_eq!(e("(a + 1").offset, 6);
        assert_eq!(e("a + 1)").offset, 5);
        assert_eq!(e("a # b").offset, 2);
        assert_eq!(e("nbt(SaddleItem, (x)").offset, 19);
        // The 65th bracket is one too many.
        let deep = format!("{}a{}", "(a + ".repeat(70), ")".repeat(70));
        assert!(e(&deep).message.contains("deep"));
        assert_eq!(e(&deep).offset, 64 * 5);
        assert!(compile(&format!("{}a", "-".repeat(64)), &mut names).is_ok());
        assert!(e(&format!("{}a", "-".repeat(65))).message.contains("deep"));
        let calls = format!("{}a{}", "abs(".repeat(65), ")".repeat(65));
        assert_eq!(e(&calls).offset, 64 * 4 + 3);
        // A long chain fails instead of folding thousands deep, somewhere inside the chain.
        let long = ["a"; 2000].join("+");
        assert!(e(&long).message.contains("long"));
        assert!((1..long.len()).contains(&e(&long).offset));
        // `a+a...` is 2n - 1 nodes: 512 terms are 1023, under the cap of 1024, and run on a
        // shallow stack; 513 terms are 1025, one too many.
        let most = expr(&["a"; 512].join("+"));
        assert_eq!(most.eval(&[1.0]), 512.0);
        assert!(most.depth <= STACK);
        assert!(e(&["a"; 513].join("+")).message.contains("long"));
    }
}
