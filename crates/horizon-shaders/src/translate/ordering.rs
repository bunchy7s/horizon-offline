//! Compute programs written against drivers that happen to run a dispatch in order. GL leaves
//! two things undefined that packs still lean on, and Apple GPUs schedule differently enough to
//! show it frame after frame:
//!
//! - An invocation reading texels of an image that other invocations of the same dispatch store
//!   (light lists read from the neighbouring work groups, reprojection or a shift in place). When
//!   every load of such an image comes before every store, no atomic function touches it and it
//!   is neither `coherent` nor `volatile`, its loads read a copy taken before the dispatch: the
//!   values the pack expects to find.
//! - Shared memory read after other invocations' atomic functions with no `barrier()` between.
//!   A barrier goes after each statement of `main` that applies one to a shared variable,
//!   directly or through a function, unless `main` returns early (a barrier then would not be
//!   reached by every invocation).
//!
//! Both only narrow what may happen to what the pack was written for, and keep the lines.

use super::scan::{is_ident, significant};
use crate::macros::lex;

const ATOMICS: [&str; 8] = [
    "atomicAdd",
    "atomicMin",
    "atomicMax",
    "atomicAnd",
    "atomicOr",
    "atomicXor",
    "atomicExchange",
    "atomicCompSwap",
];

/// The name the copy of image `name` is bound under.
pub(super) fn before_name(name: &str) -> String {
    format!("hz_before_{name}")
}

/// The images of `code` (name, declared `coherent` or `volatile`) whose loads may read the copy
/// taken before the dispatch.
pub(super) fn snapshots(code: &str, images: &[(String, bool)]) -> Vec<String> {
    let tokens = lex(code);
    let next = |i: usize| (i + 1..tokens.len()).find(|&j| significant(tokens[j]));
    // Positions of the calls taking each image as first argument.
    let calls = |name: &str, f: &dyn Fn(&str) -> bool| -> Vec<usize> {
        (0..tokens.len())
            .filter(|&i| {
                f(tokens[i])
                    && next(i).is_some_and(|o| {
                        tokens[o] == "(" && next(o).is_some_and(|n| tokens[n] == name)
                    })
            })
            .collect()
    };
    let repeats = Repeats::of(&tokens);
    images
        .iter()
        .filter(|(_, coherent)| !coherent)
        .filter(|(name, _)| {
            let loads = calls(name, &|t| t == "imageLoad");
            let stores = calls(name, &|t| t == "imageStore");
            let atomics = calls(name, &|t| t.starts_with("imageAtomic"));
            // A load repeated after a store of the same invocation (a loop around both, or a
            // function holding both run more than once) must see the store.
            let shared = loads.iter().any(|&l| {
                let around = repeats.around(l);
                stores
                    .iter()
                    .any(|&s| repeats.around(s).iter().any(|r| around.contains(r)))
            });
            // The text gives the order they run in only within one function.
            let function = loads.first().map(|&l| repeats.function(l));
            let together = loads
                .iter()
                .chain(&stores)
                .all(|&i| Some(repeats.function(i)) == function);
            !loads.is_empty()
                && !stores.is_empty()
                && atomics.is_empty()
                && !shared
                && together
                && loads.iter().max() < stores.iter().min()
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// What runs a token more than once: the loops around it, and for a token in a function, the
/// loops around its calls, or the function itself when it is called more than once.
struct Repeats {
    /// Loop bodies as token ranges, with their ids.
    loops: Vec<(std::ops::Range<usize>, usize)>,
    /// Function bodies as token ranges, with what repeats each from its calls.
    functions: Vec<(std::ops::Range<usize>, Vec<usize>)>,
}

impl Repeats {
    fn of(tokens: &[&str]) -> Self {
        let sig: Vec<usize> = (0..tokens.len())
            .filter(|&i| significant(tokens[i]))
            .collect();
        let t = |k: usize| sig.get(k).map_or("", |&i| tokens[i]);
        // `(` at sig index `k`: its `)`.
        let close = |k: usize| {
            let mut depth = 0;
            (k..sig.len()).find(|&j| {
                match t(j) {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
        };
        // A statement starting at sig index `k`: its last sig index.
        let statement = |k: usize| {
            if t(k) == "{" {
                Some(matching(&sig, tokens, k))
            } else {
                (k..sig.len()).find(|&j| t(j) == ";")
            }
        };
        let mut loops = Vec::new();
        for k in 0..sig.len() {
            let body = match t(k) {
                "for" | "while" if t(k + 1) == "(" => close(k + 1).map(|c| c + 1),
                "do" => Some(k + 1),
                _ => None,
            };
            if let Some((from, to)) = body.and_then(|b| Some((b, statement(b)?))) {
                loops.push((sig[from]..sig[to] + 1, loops.len()));
            }
        }
        let mut bodies = Vec::new();
        let mut k = 0;
        while k < sig.len() {
            if t(k) == "{" && k > 1 && t(k - 1) == ")" {
                let mut open = k - 1;
                let mut depth = 0;
                while open > 0 {
                    match t(open) {
                        ")" => depth += 1,
                        "(" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    open -= 1;
                }
                let end = matching(&sig, tokens, k);
                if open > 0
                    && is_ident(t(open - 1))
                    && !matches!(t(open - 1), "if" | "for" | "while" | "switch")
                {
                    bodies.push((t(open - 1), sig[open - 1], sig[k]..sig[end] + 1));
                    k = end + 1;
                    continue;
                }
            }
            k += 1;
        }
        let mut next_id = loops.len();
        let functions = bodies
            .iter()
            .map(|(name, defined, range)| {
                let calls: Vec<usize> = (0..sig.len())
                    .filter(|&j| t(j) == *name && t(j + 1) == "(" && sig[j] != *defined)
                    .filter(|&j| !range.contains(&sig[j]))
                    .map(|j| sig[j])
                    .collect();
                let mut repeats: Vec<usize> = calls
                    .iter()
                    .flat_map(|&c| {
                        loops
                            .iter()
                            .filter(move |(r, _)| r.contains(&c))
                            .map(|l| l.1)
                    })
                    .collect();
                if calls.len() > 1 {
                    repeats.push(next_id);
                    next_id += 1;
                }
                (range.clone(), repeats)
            })
            .collect();
        Self { loops, functions }
    }

    /// The function token `i` is in, by its place among the functions.
    fn function(&self, i: usize) -> Option<usize> {
        self.functions.iter().position(|(r, _)| r.contains(&i))
    }

    /// What repeats token `i`.
    fn around(&self, i: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .loops
            .iter()
            .filter(|(r, _)| r.contains(&i))
            .map(|l| l.1)
            .collect();
        for (r, repeats) in &self.functions {
            if r.contains(&i) {
                out.extend(repeats);
            }
        }
        out
    }
}

/// `code` with the loads of `images` reading their copies.
pub(super) fn read_before(code: &str, images: &[String]) -> String {
    let tokens = lex(code);
    let next = |i: usize| (i + 1..tokens.len()).find(|&j| significant(tokens[j]));
    let mut out: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
    for i in 0..tokens.len() {
        if tokens[i] != "imageLoad" {
            continue;
        }
        if let Some(n) = next(i)
            .filter(|&o| tokens[o] == "(")
            .and_then(next)
            .filter(|&n| images.iter().any(|m| m == tokens[n]))
        {
            out[n] = before_name(tokens[n]);
        }
    }
    out.concat()
}

/// `code` with a barrier after each statement of `hz_main` that applies an atomic function to
/// shared memory and is not followed by one; `None` when nothing changes.
pub(super) fn barriers_after_atomics(code: &str) -> Option<String> {
    let tokens = lex(code);
    let sig: Vec<usize> = (0..tokens.len())
        .filter(|&i| significant(tokens[i]))
        .collect();
    let t = |k: usize| sig.get(k).map_or("", |&i| tokens[i]);
    let shared = shared_names(&tokens, &sig);
    if shared.is_empty() {
        return None;
    }
    // Function bodies: (name, first, last) as indices into `sig`, braces included.
    let mut functions = Vec::new();
    let mut depth = 0;
    let mut k = 0;
    while k < sig.len() {
        match t(k) {
            "{" if depth == 0 && k > 0 && t(k - 1) == ")" => {
                let mut open = k - 1;
                let mut parens = 0;
                while open > 0 {
                    match t(open) {
                        ")" => parens += 1,
                        "(" => {
                            parens -= 1;
                            if parens == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    open -= 1;
                }
                let end = matching(&sig, &tokens, k);
                if open > 0 && is_ident(t(open - 1)) {
                    functions.push((t(open - 1), k, end));
                }
                k = end + 1;
                continue;
            }
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    let direct = |from: usize, to: usize| {
        (from..to).any(|k| ATOMICS.contains(&t(k)) && t(k + 1) == "(" && shared.contains(&t(k + 2)))
    };
    let mut atomic: Vec<&str> = functions
        .iter()
        .filter(|f| f.0 != "hz_main" && direct(f.1, f.2))
        .map(|f| f.0)
        .collect();
    // Functions calling those, until no more are found.
    loop {
        let more: Vec<&str> = functions
            .iter()
            .filter(|f| f.0 != "hz_main" && !atomic.contains(&f.0))
            .filter(|f| (f.1..f.2).any(|k| atomic.contains(&t(k)) && t(k + 1) == "("))
            .map(|f| f.0)
            .collect();
        if more.is_empty() {
            break;
        }
        atomic.extend(more);
    }
    let &(_, open, close) = functions.iter().find(|f| f.0 == "hz_main")?;
    if (open..close).any(|k| t(k) == "return") {
        return None;
    }
    let touches = |from: usize, to: usize| {
        direct(from, to) || (from..to).any(|k| atomic.contains(&t(k)) && t(k + 1) == "(")
    };
    // Statements of the body: each ends at a `;` or a block's `}` at its top level (an `if`
    // continues into its `else`, a `do` into its `while`).
    let mut statements = Vec::new();
    let (mut start, mut braces, mut parens) = (open + 1, 0, 0);
    for k in open + 1..close {
        match t(k) {
            "(" => parens += 1,
            ")" => parens -= 1,
            "{" => braces += 1,
            "}" => {
                braces -= 1;
                if braces == 0 && parens == 0 && !matches!(t(k + 1), "else" | "while" | ";") {
                    statements.push((start, k));
                    start = k + 1;
                }
            }
            ";" if braces == 0 && parens == 0 && t(k + 1) != "else" => {
                statements.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    let mut out: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
    let mut changed = false;
    for (n, &(from, to)) in statements.iter().enumerate() {
        let followed = statements
            .get(n + 1)
            .is_some_and(|&(next, _)| t(next) == "barrier");
        if !followed && statements.get(n + 1).is_some() && touches(from, to) {
            out[sig[to]].push_str(" barrier();");
            changed = true;
        }
    }
    changed.then(|| out.concat())
}

/// The variables declared `shared`.
fn shared_names<'t>(tokens: &[&'t str], sig: &[usize]) -> Vec<&'t str> {
    let t = |k: usize| sig.get(k).map_or("", |&i| tokens[i]);
    let mut names = Vec::new();
    for k in 0..sig.len() {
        if t(k) != "shared" {
            continue;
        }
        // `shared T a, b[N];` or `shared T[N] a;`: the identifiers after the type, outside
        // brackets, before the `;`.
        let mut j = k + 2;
        let mut brackets = 0;
        while j < sig.len() && t(j) != ";" {
            match t(j) {
                "[" => brackets += 1,
                "]" => brackets -= 1,
                w if brackets == 0 && is_ident(w) => names.push(w),
                _ => {}
            }
            j += 1;
        }
    }
    names
}

/// The `}` closing the `{` at `sig[open]`, as an index into `sig`.
fn matching(sig: &[usize], tokens: &[&str], open: usize) -> usize {
    let mut depth = 0;
    for (k, &i) in sig.iter().enumerate().skip(open) {
        match tokens[i] {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    sig.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_before_stores_read_the_copy() {
        let code = "void hz_main() { ivec4 a = imageLoad(lights, c + o); imageStore(lights, c, a); \
                    vec4 g = imageLoad(gi, c); imageStore(gi, c, g); g = imageLoad(gi, c); }";
        let images = [("lights".to_owned(), false), ("gi".to_owned(), false)];
        assert_eq!(snapshots(code, &images), ["lights"]);
        let read = read_before(code, &["lights".into()]);
        assert!(read.contains("imageLoad(hz_before_lights, c + o)"));
        assert!(read.contains("imageStore(lights, c, a)"));
    }

    #[test]
    fn loads_repeated_after_stores_stay() {
        let images = [("v".to_owned(), false)];
        let looped = "void hz_main() { for (int i = 0; i < 4; i++) { vec4 a = imageLoad(v, p); \
                      imageStore(v, p, a + 1.0); } }";
        assert!(snapshots(looped, &images).is_empty());
        let unbraced = "void hz_main() { while (k < 4) step(k++); }\n\
                        void step(int k) { vec4 a = imageLoad(v, p); imageStore(v, p, a); }";
        assert!(snapshots(unbraced, &images).is_empty());
        let twice = "void step() { vec4 a = imageLoad(v, p); imageStore(v, p, a); }\n\
                     void hz_main() { step(); step(); }";
        assert!(snapshots(twice, &images).is_empty());
        let once = "void step() { vec4 a = imageLoad(v, p + o); imageStore(v, p, a); }\n\
                    void hz_main() { for (int i = 0; i < 2; i++) { o += i; } step(); }";
        assert_eq!(snapshots(once, &images), ["v"]);
        let helper = "vec4 get(ivec2 p) { return imageLoad(v, p); }\n\
                      void hz_main() { imageStore(v, c, x); vec4 y = get(c); }";
        assert!(snapshots(helper, &images).is_empty());
    }

    #[test]
    fn coherent_and_atomic_images_stay() {
        let code = "void hz_main() { int a = imageLoad(v, c).r; imageStore(v, c, ivec4(a)); \
                    int b = imageLoad(o, c).r; imageStore(o, c, ivec4(b)); imageAtomicAdd(o, c, 1); }";
        let images = [("v".to_owned(), true), ("o".to_owned(), false)];
        assert!(snapshots(code, &images).is_empty());
    }

    #[test]
    fn a_barrier_follows_shared_atomics() {
        let code = "shared int counts[4]; shared int total;\n\
                    void add(int k) { atomicAdd(total, k); }\n\
                    void hz_main() {\n\
                    if (a) { for (int k = 0; k < 4; k++) atomicMax(counts[k], 1); }\n\
                    int x = counts[0];\n\
                    add(x);\n\
                    barrier();\n\
                    int y = total;\n\
                    }";
        let out = barriers_after_atomics(code).unwrap();
        assert_eq!(out.lines().count(), code.lines().count());
        assert!(out.contains("atomicMax(counts[k], 1); } barrier();"));
        assert!(!out.contains("add(x); barrier();"));
    }

    #[test]
    fn a_barrier_does_not_split_if_from_else() {
        let code = "shared int total;\nvoid hz_main() {\n if (x) atomicAdd(total, 1); else y = 1;\n int z = total;\n}";
        let out = barriers_after_atomics(code).unwrap();
        assert!(out.contains("else y = 1; barrier();"), "{out}");
        assert!(!out.contains("1); barrier(); else"), "{out}");
    }

    #[test]
    fn no_barrier_when_main_returns_early() {
        let code = "shared int total;\nvoid hz_main() { if (a) return; atomicAdd(total, 1); int y = total; }";
        assert!(barriers_after_atomics(code).is_none());
    }
}
