//! Resolve the scalar signed/unsigned clamp case still ambiguous in Naga.
//! Only an unambiguous `int` value with two unsigned literal bounds is changed.
use crate::macros::lex;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn rewrite(text: &str) -> String {
    let tokens = lex(text);
    let words: Vec<_> = tokens
        .iter()
        .copied()
        .filter(|t| !t.trim().is_empty())
        .collect();
    let mut types: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for pair in words.windows(2) {
        if matches!(
            pair[0],
            "int"
                | "uint"
                | "float"
                | "double"
                | "bool"
                | "vec2"
                | "vec3"
                | "vec4"
                | "ivec2"
                | "ivec3"
                | "ivec4"
                | "uvec2"
                | "uvec3"
                | "uvec4"
        ) {
            types.entry(pair[1]).or_default().insert(pair[0]);
        }
    }
    // A pack may overload clamp itself; let upstream resolve its own function.
    if types.contains_key("clamp") {
        return text.to_owned();
    }
    let significant: Vec<usize> = (0..tokens.len())
        .filter(|&i| !tokens[i].trim().is_empty())
        .collect();
    let mut replacements = BTreeMap::new();
    for w in significant.windows(8) {
        let t: Vec<_> = w.iter().map(|&i| tokens[i]).collect();
        if t[0] != "clamp" || t[1] != "(" || t[3] != "," || t[5] != "," || t[7] != ")" {
            continue;
        }
        let uint = |t: &str| {
            t.strip_suffix(['u', 'U'])
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        };
        if uint(t[4])
            && uint(t[6])
            && types
                .get(t[2])
                .is_some_and(|s| s.len() == 1 && s.contains("int"))
        {
            // GLSL promotes the signed argument to the unsigned overload. Do not
            // narrow the bounds to int, which changes negative-value behavior.
            replacements.insert(w[2], format!("uint({})", t[2]));
        }
    }
    tokens
        .iter()
        .enumerate()
        .map(|(i, t)| replacements.get(&i).map_or(*t, String::as_str))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_value_promotes_without_changing_unsigned_bounds_or_lines() {
        let input = "int index=3;\nuint n=clamp(index, 0u, 47u);";
        let custom =
            "uint clamp(int value,uint lo,uint hi){return lo;}int value;clamp(value,0u,47u)";
        assert_eq!(rewrite(custom), custom);
        let output = rewrite(input);
        assert!(output.contains("clamp(uint(index), 0u, 47u)"));
        assert_eq!(input.matches('\n').count(), output.matches('\n').count());
        assert_eq!(
            rewrite("float index; clamp(index,0u,47u)"),
            "float index; clamp(index,0u,47u)"
        );
        assert_eq!(
            rewrite("int index;void f(){float index;}clamp(index,0u,47u)"),
            "int index;void f(){float index;}clamp(index,0u,47u)"
        );
    }
}
