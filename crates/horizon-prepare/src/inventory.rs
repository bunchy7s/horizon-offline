//! Class inventories of the imported jars (tools/lab.py's `inventory` and `ClassReader`): each
//! class's SourceFile and method count, read from the bytecode; no Java text is published.

use crate::pyjson::Value;
use std::path::Path;

struct ClassReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl ClassReader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let value = self
            .data
            .get(self.offset..self.offset + count)
            .ok_or("Truncated class file")?;
        self.offset += count;
        Ok(value)
    }

    fn number(&mut self, count: usize) -> Result<usize, String> {
        Ok(self
            .take(count)?
            .iter()
            .fold(0usize, |n, b| (n << 8) | *b as usize))
    }

    fn attributes(&mut self) -> Result<Vec<(usize, Vec<u8>)>, String> {
        let count = self.number(2)?;
        let mut attributes = Vec::with_capacity(count);
        for _ in 0..count {
            let name = self.number(2)?;
            let length = self.number(4)?;
            attributes.push((name, self.take(length)?.to_vec()));
        }
        Ok(attributes)
    }

    /// `(SourceFile, method count)`.
    fn metadata(&mut self) -> Result<(Option<String>, usize), String> {
        if self.take(4)? != b"\xca\xfe\xba\xbe" {
            return Err("Invalid class file magic".into());
        }
        self.take(4)?; // Minor and major version.
        let mut pool: Vec<Option<String>> = vec![None; self.number(2)?];
        let mut i = 1;
        while i < pool.len() {
            let tag = self.number(1)?;
            if tag == 1 {
                let length = self.number(2)?;
                pool[i] = Some(String::from_utf8_lossy(self.take(length)?).into_owned());
            } else {
                let size = match tag {
                    3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
                    5 | 6 => 8,
                    7 | 8 | 16 | 19 | 20 => 2,
                    15 => 3,
                    _ => return Err(format!("Unknown constant pool tag: {tag}")),
                };
                self.take(size)?;
            }
            i += if tag == 5 || tag == 6 { 2 } else { 1 };
        }
        self.take(6)?; // Access, this class, super class.
        let interfaces = self.number(2)?;
        self.take(interfaces * 2)?;
        for _ in 0..self.number(2)? {
            self.take(6)?; // Fields.
            self.attributes()?;
        }
        let methods = self.number(2)?;
        for _ in 0..methods {
            self.take(6)?;
            self.attributes()?;
        }
        let mut source = None;
        for (name, data) in self.attributes()? {
            if pool.get(name).and_then(Option::as_deref) == Some("SourceFile") {
                let [high, low] = data[..] else {
                    return Err("Malformed SourceFile attribute".into());
                };
                let index = ((high as usize) << 8) | low as usize;
                source = Some(
                    pool.get(index)
                        .cloned()
                        .flatten()
                        .ok_or("SourceFile points outside the constant pool")?,
                );
            }
        }
        Ok((source, methods))
    }
}

/// PurePosixPath(name).parent as text.
fn parent(name: &str) -> &str {
    match name.trim_end_matches('/').rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent,
        Some(_) => "/",
        None => ".",
    }
}

pub fn inventory(jar: &Path) -> Result<Value, String> {
    let mut archive = crate::open_zip(jar)?;
    let mut classes = Vec::new();
    let mut packages: Vec<(String, usize)> = Vec::new();
    let mut sources = std::collections::BTreeSet::new();
    let mut method_count = 0;
    for index in 0..archive.len() {
        let mut item = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = item.name().map_err(|e| e.to_string())?.into_owned();
        if !name.ends_with(".class") {
            continue;
        }
        let size = item.size();
        let mut data = Vec::with_capacity(size as usize);
        std::io::Read::read_to_end(&mut item, &mut data).map_err(|e| e.to_string())?;
        let (source, methods) = ClassReader {
            data: &data,
            offset: 0,
        }
        .metadata()
        .map_err(|e| format!("{name}: {e}"))?;
        let file = name.rsplit('/').next().unwrap();
        let source = source.filter(|s| !s.is_empty()).unwrap_or_else(|| {
            let stem = file.split('$').next().unwrap();
            format!("{}.java", stem.strip_suffix(".class").unwrap_or(stem))
        });
        if source.contains('/') || source.contains('\\') || source == "." {
            return Err(format!("Unsafe SourceFile attribute in {name}"));
        }
        let directory = parent(&name);
        let source_path = if directory == "." {
            source
        } else {
            format!("{directory}/{source}")
        };
        match packages.iter_mut().find(|(p, _)| p == directory) {
            Some(entry) => entry.1 += 1,
            None => packages.push((directory.to_string(), 1)),
        }
        sources.insert(source_path.clone());
        method_count += methods;
        classes.push(Value::Object(vec![
            ("class".into(), Value::Str(name)),
            ("source".into(), Value::Str(source_path)),
            ("bytecode_bytes".into(), Value::Int(size.to_string())),
            ("methods".into(), Value::Int(methods.to_string())),
        ]));
    }
    // Counter.most_common: by count, ties in first-seen order.
    packages.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    Ok(Value::Object(vec![
        (
            "jar_sha256".into(),
            Value::Str(crate::digest(jar, "sha256")?),
        ),
        ("class_count".into(), Value::Int(classes.len().to_string())),
        (
            "expected_source_files".into(),
            Value::Array(sources.into_iter().map(Value::Str).collect()),
        ),
        ("method_count".into(), Value::Int(method_count.to_string())),
        (
            "packages".into(),
            Value::Object(
                packages
                    .into_iter()
                    .map(|(p, n)| (p, Value::Int(n.to_string())))
                    .collect(),
            ),
        ),
        ("classes".into(), Value::Array(classes)),
    ]))
}
