//! The arithmetic of `#if` and `#elif` (spec 1.5, GLSL 4.60 §3.3): `defined`, integer
//! literals, macros expanded to their values, and the C operators with C precedence. Float
//! literals count as numbers too, as GL drivers take them (a pack's `#define AMOUNT 0.`).
//! An identifier that is not a macro counts as 0, as in C: GLSL would reject the line, and a
//! pack would then lose a whole section of its settings over one unknown name.

use crate::Macros;
use crate::macros::{is_ident, is_ident_start};

/// The value of a condition; `Err` says what is wrong with it.
pub fn eval_condition(expr: &str, macros: &Macros) -> Result<i64, String> {
    let tokens = tokens(&resolve_defined(expr, macros)?, macros)?;
    let real = tokens
        .iter()
        .any(|t| matches!(t, Token::Num(n) if n.fract() != 0.0))
        || expr.contains('.');
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        real,
    };
    let value = parser.binary(0)?;
    match parser.tokens.get(parser.at) {
        // A fraction is true, as any value but zero.
        None if value != 0.0 && value.abs() < 1.0 => Ok(value.signum() as i64),
        None => Ok(value as i64),
        Some(t) => Err(format!("unexpected `{}`", t.text())),
    }
}

/// The text after a line's `#`, split into the directive word and the rest without its `//`
/// comment. `None` when the first word is followed by something other than white space, `//` or
/// the end of the line (`#ifdefault=1` is a comment, not `#ifdef`).
pub(crate) fn split_directive(after_hash: &str) -> Option<(&str, &str)> {
    let line = after_hash.trim_start();
    let len = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (word, rest) = line.split_at(len);
    let ok = rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with("//");
    ok.then(|| (word, rest.split("//").next().unwrap_or("").trim()))
}

/// One level of `#if` ... `#endif`.
struct Branch {
    /// This branch's lines are read.
    on: bool,
    /// A branch of this `#if` was already taken (the rest are skipped).
    taken: bool,
    /// The enclosing level is read.
    outer: bool,
    seen_else: bool,
}

/// The state of nested conditional directives.
#[derive(Default)]
pub(crate) struct Conditionals(Vec<Branch>);

impl Conditionals {
    /// Lines here are read.
    pub(crate) fn active(&self) -> bool {
        self.0.last().is_none_or(|b| b.on)
    }

    /// An `#if` is still open.
    pub(crate) fn open(&self) -> bool {
        !self.0.is_empty()
    }

    /// Applies `#word rest` when it is a conditional directive (`Ok(false)` when it is not). A
    /// condition that can't be evaluated reads as false and returns why.
    pub(crate) fn apply(
        &mut self,
        word: &str,
        rest: &str,
        macros: &Macros,
    ) -> Result<bool, String> {
        let on = self.active();
        match word {
            "if" | "ifdef" | "ifndef" => {
                let value = if on {
                    test(word, rest, macros)
                } else {
                    Ok(false)
                };
                let v = *value.as_ref().unwrap_or(&false);
                self.0.push(Branch {
                    on: v,
                    taken: v,
                    outer: on,
                    seen_else: false,
                });
                value.map(|_| true)
            }
            "elif" | "else" => {
                let b = self
                    .0
                    .last_mut()
                    .ok_or_else(|| format!("`#{word}` without `#if`"))?;
                if b.seen_else {
                    return Err(format!("`#{word}` after `#else`"));
                }
                b.seen_else = word == "else";
                let take = b.outer && !b.taken;
                let value = match word {
                    "else" => Ok(take),
                    _ if take => test("if", rest, macros),
                    _ => Ok(false),
                };
                b.on = *value.as_ref().unwrap_or(&false);
                b.taken |= b.on;
                value.map(|_| true)
            }
            "endif" => self
                .0
                .pop()
                .map(|_| true)
                .ok_or_else(|| "`#endif` without `#if`".into()),
            _ => Ok(false),
        }
    }
}

fn test(word: &str, rest: &str, macros: &Macros) -> Result<bool, String> {
    let name = rest.split_whitespace().next().unwrap_or("");
    match word {
        "ifdef" => Ok(macros.is_defined(name)),
        "ifndef" => Ok(!macros.is_defined(name)),
        _ => eval_condition(rest, macros)
            .map(|v| v != 0)
            .map_err(|e| format!("`#{word} {rest}`: {e}; read as false")),
    }
}

/// `defined X` and `defined(X)` become `1` or `0` before anything is expanded.
fn resolve_defined(expr: &str, macros: &Macros) -> Result<String, String> {
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(at) = find_word(rest, "defined") {
        out.push_str(&rest[..at]);
        let after = rest[at + "defined".len()..].trim_start();
        let (inner, tail) = match after.strip_prefix('(') {
            Some(open) => {
                let close = open.find(')').ok_or("`defined(` without `)`")?;
                (open[..close].trim(), &open[close + 1..])
            }
            None => {
                let len = after.find(|c: char| !is_ident(c)).unwrap_or(after.len());
                (&after[..len], &after[len..])
            }
        };
        if inner.is_empty() || !inner.starts_with(is_ident_start) || !inner.chars().all(is_ident) {
            return Err("`defined` needs a macro name".into());
        }
        out.push_str(if macros.is_defined(inner) {
            " 1 "
        } else {
            " 0 "
        });
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// Where `word` starts in `text` as a whole identifier.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let before = text[..i].chars().next_back().is_some_and(is_ident);
        let after = text[i + word.len()..].chars().next().is_some_and(is_ident);
        if !before && !after {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Num(f64),
    Op(&'static str),
}

impl Token {
    fn text(&self) -> String {
        match self {
            Token::Num(n) => n.to_string(),
            Token::Op(op) => (*op).to_owned(),
        }
    }
}

const OPS: [&str; 22] = [
    "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "(", ")", "!", "~", "+", "-", "*", "/", "%",
    "<", ">", "&", "^", "|",
];

fn tokens(expr: &str, macros: &Macros) -> Result<Vec<Token>, String> {
    let expanded = macros.expand(expr);
    let mut out = Vec::new();
    let mut rest = expanded.trim_start();
    while let Some(c) = rest.chars().next() {
        let fraction = c == '.' && rest[1..].starts_with(|c: char| c.is_ascii_digit());
        if c.is_ascii_digit() || fraction {
            let len = rest
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(rest.len());
            if fraction || rest[len..].starts_with('.') {
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
                    .unwrap_or(rest.len());
                let text = rest[..len].trim_end_matches(['f', 'F']);
                let n = text
                    .parse::<f64>()
                    .map_err(|_| format!("`{text}` is not a number"))?;
                out.push(Token::Num(n));
                rest = &rest[len..];
            } else {
                out.push(Token::Num(int_literal(&rest[..len])? as f64));
                rest = &rest[len..];
            }
        } else if is_ident_start(c) {
            // Not a macro: 0, as the C preprocessor counts it.
            let len = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
            out.push(Token::Num(0.0));
            rest = &rest[len..];
        } else {
            let op = OPS
                .iter()
                .find(|op| rest.starts_with(**op))
                .ok_or_else(|| format!("unexpected `{c}`"))?;
            out.push(Token::Op(op));
            rest = &rest[op.len()..];
        }
        rest = rest.trim_start();
    }
    Ok(out)
}

/// Decimal, `0x` hexadecimal or leading-zero octal, with an optional `u` suffix.
fn int_literal(text: &str) -> Result<i64, String> {
    let digits = text.strip_suffix(['u', 'U']).unwrap_or(text);
    let parsed = if let Some(hex) = digits.strip_prefix("0x").or(digits.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        i64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse()
    };
    parsed.map_err(|_| format!("`{text}` is not an integer"))
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    /// A float literal appears: `/` and `%` divide as floats.
    real: bool,
}

/// Binary operators from the loosest to the tightest (GLSL §5.1 / §3.3).
const LEVELS: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

impl Parser<'_> {
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).copied();
        self.at += 1;
        t
    }

    fn binary(&mut self, level: usize) -> Result<f64, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(&Token::Op(op)) = self.tokens.get(self.at) {
            if !LEVELS[level].contains(&op) {
                break;
            }
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = apply(op, left, right, self.real)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::Op("(")) => {
                let value = self.binary(0)?;
                match self.next() {
                    Some(Token::Op(")")) => Ok(value),
                    _ => Err("missing `)`".into()),
                }
            }
            Some(Token::Op("-")) => Ok(-self.unary()?),
            Some(Token::Op("+")) => self.unary(),
            Some(Token::Op("!")) => Ok(f64::from(u8::from(self.unary()? == 0.0))),
            Some(Token::Op("~")) => Ok(!(self.unary()? as i64) as f64),
            Some(t) => Err(format!("unexpected `{}`", t.text())),
            None => Err("the condition ends too early".into()),
        }
    }
}

fn apply(op: &str, a: f64, b: f64, real: bool) -> Result<f64, String> {
    let truth = |v: bool| f64::from(u8::from(v));
    let (i, j) = (a as i64, b as i64);
    Ok(match op {
        "||" => truth(a != 0.0 || b != 0.0),
        "&&" => truth(a != 0.0 && b != 0.0),
        "|" => (i | j) as f64,
        "^" => (i ^ j) as f64,
        "&" => (i & j) as f64,
        "==" => truth(a == b),
        "!=" => truth(a != b),
        "<" => truth(a < b),
        ">" => truth(a > b),
        "<=" => truth(a <= b),
        ">=" => truth(a >= b),
        "<<" => i.wrapping_shl(j as u32) as f64,
        ">>" => i.wrapping_shr(j as u32) as f64,
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" | "%" if b == 0.0 => return Err("division by zero".into()),
        "/" if real => a / b,
        "%" if real => a % b,
        "/" => i.wrapping_div(j) as f64,
        _ => i.wrapping_rem(j) as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<i64, String> {
        let mut m = Macros::standard();
        m.define("SHADOW_RES", "2048");
        m.define("WAVES", "");
        m.define("QUALITY", "SHADOW_RES / 1024");
        eval_condition(expr, &m)
    }

    #[test]
    fn precedence_and_operators_follow_c() {
        assert_eq!(eval("1 + 2 * 3"), Ok(7));
        assert_eq!(eval("(1 + 2) * 3"), Ok(9));
        assert_eq!(eval("1 << 2 + 1"), Ok(8));
        assert_eq!(eval("6 & 3 == 3"), Ok(0));
        assert_eq!(eval("1 || 0 && 0"), Ok(1));
        assert_eq!(eval("-7 / 2"), Ok(-3));
        assert_eq!(eval("-7 % 2"), Ok(-1));
        assert_eq!(eval("!0 + ~0"), Ok(0));
        assert_eq!(eval("0x10 + 010 + 3u"), Ok(27));
    }

    #[test]
    fn macros_and_defined() {
        assert_eq!(eval("MC_VERSION >= 11300"), Ok(1));
        assert_eq!(eval("MC_VERSION <= 12111"), Ok(0));
        assert_eq!(eval("QUALITY == 2"), Ok(1));
        assert_eq!(eval("defined WAVES && defined(SHADOW_RES)"), Ok(1));
        assert_eq!(eval("!defined NOPE"), Ok(1));
        assert_eq!(eval("NOPE == 0"), Ok(1));
        assert_eq!(eval("undefinedFeature"), Ok(0));
    }

    /// Float values, as GL drivers read them (I Like Vanilla's `#if AMBIENT_CEL_AMOUNT != 0` with
    /// `#define AMBIENT_CEL_AMOUNT 0.0`).
    #[test]
    fn float_values_count() {
        assert_eq!(eval("0. != 0"), Ok(0));
        assert_eq!(eval("4. != 0"), Ok(1));
        assert_eq!(eval("0.7 > 0.5"), Ok(1));
        assert_eq!(eval(".25 * 4.0 == 1"), Ok(1));
        assert_eq!(eval("1.5f"), Ok(1));
        assert_eq!(eval("7 / 2"), Ok(3));
        assert_eq!(eval("SHADOW_RES == 2048.0"), Ok(1));
    }

    #[test]
    fn bad_conditions_say_why() {
        assert!(eval("1 / 0").is_err());
        assert!(eval("SHADOW_RES == 0.5 0").is_err());
        assert!(eval("(1 + 2").is_err());
        assert!(eval("1 2").is_err());
        assert!(eval("").is_err());
        assert!(eval("defined").is_err());
        assert!(eval("1 @ 2").is_err());
    }
}
