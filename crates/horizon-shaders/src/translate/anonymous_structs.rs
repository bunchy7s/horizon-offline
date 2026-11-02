//! Struct types without a name. GLSL lets a declaration define its struct type in place without
//! naming it (GLSL 4.60, 4.1.8: `shared struct { uint sum; uint tile[64]; } lds;`); naga's
//! frontend wants a name there ("Expected identifier, found LeftBrace"). Each such type gets
//! one, `hz_struct<n>`; nothing else changes.

use super::scan::significant;
use crate::macros::lex;

/// `code` with a name for each struct type declared without one, or `None` when it has none.
pub(super) fn name(code: &str) -> Option<String> {
    let tokens = lex(code);
    let mut out = String::with_capacity(code.len() + 64);
    let mut named = 0;
    for (i, t) in tokens.iter().enumerate() {
        out.push_str(t);
        let opens = (i + 1..tokens.len())
            .find(|&j| significant(tokens[j]))
            .is_some_and(|j| tokens[j] == "{");
        if *t == "struct" && opens {
            out.push_str(&format!(" hz_struct{named}"));
            named += 1;
        }
    }
    (named > 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn struct_types_without_a_name_get_one() {
        let code = "shared struct {\n    uint sum;\n} lds;\nstruct Light { vec3 c; };\n\
                    struct\n{ float a; } x, y[2];\n";
        assert_eq!(
            name(code).as_deref(),
            Some(
                "shared struct hz_struct0 {\n    uint sum;\n} lds;\nstruct Light { vec3 c; };\n\
                 struct hz_struct1\n{ float a; } x, y[2];\n"
            )
        );
        assert_eq!(name("struct Light { vec3 c; };\n"), None);
    }
}
