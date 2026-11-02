//! `in` and `out` interface blocks as plain varyings. GLSL lets a block's members carry their
//! own interpolation and layout qualifiers (GLSL 4.60, 4.3.9: `out VertexData { flat int id;
//! vec2 uv; } vOut;`); naga's frontend refuses a qualifier there ("Expected Void, Struct or a
//! type, found Interpolation(Flat)"). Each member of such a block becomes a varying of its own,
//! `hz_<block>_<member>` (the name the next stage's block gives it too: stages match blocks by
//! block name and members by name), and `instance.member` reads it. A block without an instance
//! name keeps its members' names. Of a block with an instance array (a geometry stage's input,
//! `in VertexData { vec2 uv; } vIn[];`), each member becomes an array, `vIn[i].uv` reading
//! `hz_VertexData_uv[i]`. Blocks whose instance is used whole are left as written. Lines stay
//! where they were.

use super::scan::significant;
use crate::macros::lex;

/// Qualifiers a member may carry before its type.
const MEMBER_QUALIFIERS: [&str; 11] = [
    "flat",
    "smooth",
    "noperspective",
    "centroid",
    "sample",
    "invariant",
    "precise",
    "highp",
    "mediump",
    "lowp",
    "layout",
];

/// A block found at global scope: tokens `start..=end` (its `;`), its storage, the members'
/// declarations (token ranges, each without its `;`) with their names, and its instance name.
struct Block {
    start: usize,
    open: usize,
    close: usize,
    end: usize,
    storage: &'static str,
    name: String,
    members: Vec<(usize, Vec<(usize, String)>)>,
    instance: Option<(usize, String)>,
    /// The instance's array suffix (`[]`, `[3]`), empty when it is not an array.
    array: String,
}

/// `code` with its plain `in`/`out` interface blocks as varyings, or `None` when it has none.
pub(super) fn flatten(code: &str) -> Option<String> {
    flatten_as(code, "")
}

/// `flatten`, the members of an `in` block with an instance named `hz_<inputs><block>_<member>`:
/// a geometry stage's input and output blocks may share their name (GLSL 4.60, 4.3.9), and the
/// geometry stage reads its inputs under names of its own.
pub(super) fn flatten_as(code: &str, inputs: &str) -> Option<String> {
    let mut tokens: Vec<String> = lex(code).into_iter().map(str::to_owned).collect();
    let blocks = find_blocks(&tokens);
    if blocks.is_empty() {
        return None;
    }
    let mut changed = false;
    for b in blocks {
        let inside = |i: usize| (b.start..=b.end).contains(&i);
        let prefix = if b.storage == "in" { inputs } else { "" };
        let renamed = |member: &str| match &b.instance {
            Some(_) => format!("hz_{prefix}{}_{member}", b.name),
            None => member.to_owned(),
        };
        // Every use of the instance outside the block must be `instance.member`, or
        // `instance[i].member` for an instance array.
        let mut reads = Vec::new();
        if let Some((_, instance)) = &b.instance {
            let mut whole = false;
            for i in (0..tokens.len()).filter(|&i| !inside(i) && tokens[i] == *instance) {
                let index = match b.array.is_empty() {
                    true => Some(i),
                    false => next_significant(&tokens, i + 1)
                        .filter(|&o| tokens[o] == "[")
                        .and_then(|o| closing(&tokens, o)),
                };
                let member = index
                    .and_then(|x| next_significant(&tokens, x + 1))
                    .filter(|&d| tokens[d] == ".")
                    .and_then(|d| next_significant(&tokens, d + 1));
                let known = member.filter(|&m| {
                    b.members
                        .iter()
                        .any(|(_, names)| names.iter().any(|(_, n)| *n == tokens[m]))
                });
                match (index, known) {
                    (Some(x), Some(m)) => reads.push((i, x, m)),
                    _ => whole = true,
                }
            }
            if whole {
                continue;
            }
        }
        for (i, index, member) in reads {
            tokens[i] = renamed(&tokens[member]);
            // `.member` goes, with the spaces around it but not its line breaks; an index stays
            // (`v[i].m` is `hz_B_m[i]`).
            for t in &mut tokens[index + 1..=member] {
                t.retain(|c| c == '\n');
            }
        }
        for t in &mut tokens[b.start..=b.open] {
            blank(t);
        }
        for t in &mut tokens[b.close..=b.end] {
            blank(t);
        }
        for (first, names) in &b.members {
            tokens[*first] = format!("{} {}", b.storage, tokens[*first]);
            for (at, name) in names {
                // Of an instance array, each member is an array of the instance's size, outermost.
                tokens[*at] = renamed(name) + &b.array;
            }
        }
        changed = true;
    }
    changed.then(|| tokens.concat())
}

/// Spaces for every character of `t` but its line breaks.
fn blank(t: &mut String) {
    *t = t
        .chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect();
}

fn next_significant(tokens: &[String], from: usize) -> Option<usize> {
    (from..tokens.len()).find(|&i| significant(&tokens[i]))
}

/// The `]` that closes the `[` at `open`.
fn closing(tokens: &[String], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, t) in tokens.iter().enumerate().skip(open) {
        match t.as_str() {
            "[" => depth += 1,
            "]" => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `in`/`out` interface blocks at global scope that can be flattened.
fn find_blocks(tokens: &[String]) -> Vec<Block> {
    let mut out = Vec::new();
    let (mut start, mut depth, mut parens) = (0, 0usize, 0usize);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "(" => parens += 1,
            ")" => parens = parens.saturating_sub(1),
            "{" if depth == 0 && parens == 0 => match block(tokens, start, i) {
                Some(b) => {
                    i = b.end;
                    start = b.end + 1;
                    out.push(b);
                }
                None => depth = 1,
            },
            "{" => depth += 1,
            "}" if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    start = i + 1;
                }
            }
            ";" if depth == 0 && parens == 0 => start = i + 1,
            _ => {}
        }
        i += 1;
    }
    out
}

/// The block whose declaration runs from token `start` to its `{` at `open`, if it is a plain
/// `[layout(...)] in|out Name { members } [instance];`.
fn block(tokens: &[String], start: usize, open: usize) -> Option<Block> {
    let head: Vec<usize> = (start..open).filter(|&i| significant(&tokens[i])).collect();
    let (&name_at, rest) = head.split_last()?;
    let (&storage_at, prefix) = rest.split_last()?;
    let storage = match tokens[storage_at].as_str() {
        "in" => "in",
        "out" => "out",
        _ => return None,
    };
    // Only a `layout(...)` may come before the storage.
    let prefix: Vec<&str> = prefix.iter().map(|&i| tokens[i].as_str()).collect();
    if !(prefix.is_empty() || prefix[0] == "layout" && prefix.last() == Some(&")")) {
        return None;
    }
    let name = tokens[name_at].clone();
    if !super::scan::is_ident(&name) {
        return None;
    }
    // Members to the matching `}`: `[qualifiers] type[...] name[...], name[...];`.
    let mut members = Vec::new();
    let mut i = open + 1;
    loop {
        let first = next_significant(tokens, i)?;
        if tokens[first] == "}" {
            i = first;
            break;
        }
        let end = (first..tokens.len()).find(|&j| tokens[j] == ";")?;
        let words: Vec<usize> = (first..end).filter(|&j| significant(&tokens[j])).collect();
        if words
            .iter()
            .any(|&j| matches!(tokens[j].as_str(), "{" | "}"))
        {
            return None;
        }
        // Past the qualifiers (and a layout's parentheses) to the type.
        let mut k = 0;
        while let Some(&w) = words.get(k) {
            if tokens[w] == "layout" {
                k += 1;
                let mut level = 0;
                while let Some(&p) = words.get(k) {
                    k += 1;
                    match tokens[p].as_str() {
                        "(" => level += 1,
                        ")" => {
                            level -= 1;
                            if level == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            } else if MEMBER_QUALIFIERS.contains(&tokens[w].as_str()) {
                k += 1;
            } else {
                break;
            }
        }
        // The type, its array suffix, then the declarators.
        k += 1;
        let mut names = Vec::new();
        let mut brackets = 0;
        let mut expect_name = true;
        for &w in &words[k..] {
            match tokens[w].as_str() {
                "[" => brackets += 1,
                "]" => brackets -= 1,
                "," if brackets == 0 => expect_name = true,
                t if brackets == 0 && expect_name && super::scan::is_ident(t) => {
                    names.push((w, t.to_owned()));
                    expect_name = false;
                }
                _ => {}
            }
        }
        if names.is_empty() {
            return None;
        }
        members.push((first, names));
        i = end + 1;
    }
    let close = i;
    // `} [instance [array]] ;`
    let after: Vec<usize> = (close + 1..tokens.len())
        .take_while(|&j| tokens[j] != ";")
        .filter(|&j| significant(&tokens[j]))
        .collect();
    let end = (close + 1..tokens.len()).find(|&j| tokens[j] == ";")?;
    let (instance, array) = match after.as_slice() {
        [] => (None, String::new()),
        // `vIn[]`, `vIn[3]`: a geometry stage's input (GLSL 4.60, 4.3.9).
        [n, rest @ ..]
            if super::scan::is_ident(&tokens[*n])
                && (rest.is_empty() || tokens[rest[0]] == "[" && tokens[*rest.last()?] == "]") =>
        {
            let array = rest.iter().map(|&j| tokens[j].as_str()).collect();
            (Some((*n, tokens[*n].clone())), array)
        }
        _ => return None,
    };
    if members.is_empty() {
        return None;
    }
    Some(Block {
        start,
        open,
        close,
        end,
        storage,
        name,
        members,
        instance,
        array,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_become_varyings_on_their_lines() {
        let code = "out VertexData {\n    flat int id;\n    vec2 uv, st[2];\n} vOut;\n\
                    void main() { vOut.id = 1; vOut . uv = vec2(0.0); vOut.st[1] = vOut.uv; }\n";
        let out = flatten(code).unwrap();
        assert_eq!(out.lines().count(), code.lines().count());
        let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(
            squash(&out),
            squash(
                "out flat int hz_VertexData_id; out vec2 hz_VertexData_uv, hz_VertexData_st[2]; \
                 void main() { hz_VertexData_id = 1; hz_VertexData_uv = vec2(0.0); \
                 hz_VertexData_st[1] = hz_VertexData_uv; }"
            )
        );
    }

    #[test]
    fn a_block_without_an_instance_keeps_its_member_names() {
        let code = "in VertexData { flat int id; };\nvoid main() { float x = float(id); }\n";
        let out = flatten(code).unwrap();
        assert_eq!(
            out.split_whitespace().collect::<Vec<_>>().join(" "),
            "in flat int id; void main() { float x = float(id); }"
        );
    }

    #[test]
    fn an_instance_array_makes_each_member_an_array() {
        let code = "in VertexData {\n    flat int id;\n    vec2 st[2];\n} vIn[];\n\
                    void main() { int i = vIn[0].id; vec2 a = vIn[i + 1] . st[1]; }\n";
        let out = flatten(code).unwrap();
        assert_eq!(out.lines().count(), code.lines().count());
        assert_eq!(
            out.split_whitespace().collect::<Vec<_>>().join(" "),
            "in flat int hz_VertexData_id[]; in vec2 hz_VertexData_st[][2]; void main() { \
             int i = hz_VertexData_id[0]; vec2 a = hz_VertexData_st[i + 1][1]; }"
        );
    }

    #[test]
    fn other_blocks_stay_as_written() {
        for code in [
            // Uniform and storage blocks, an instance used whole.
            "uniform Frame { float t; } frame;\nvoid main() {}\n",
            "buffer Data { float v[]; } data;\nvoid main() {}\n",
            "in VertexData { flat int id; } vIn[];\nvoid f(VertexData d) {}\nvoid main() { f(vIn[0]); }\n",
            "out VertexData { flat int id; } vOut;\nvoid f(int x) {}\nvoid main() { vOut = vOut; }\n",
        ] {
            assert_eq!(flatten(code), None, "{code}");
        }
    }
}
