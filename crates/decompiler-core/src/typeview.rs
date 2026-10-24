//! dnSpy-style type documents: one C# view of a whole type with usings, its namespace,
//! token comments and every member body. Built from the export model; nothing executes.
use crate::{
    assembly::{Assembly, visibility},
    attributes,
    cil::Operand,
    emit::SourceReference,
    error::{Error, Result},
    export::{self, MemberSource, TypeDeclaration},
    resolution::Accessor,
};
use serde::Serialize;
use std::collections::HashSet;
#[derive(Debug, Serialize)]
pub struct TypeSource {
    pub text: String,
    /// Navigation ranges in UTF-16 units, matching the code viewer.
    pub references: Vec<SourceReference>,
    /// True when a member stayed in IL or the view hit its size limit.
    pub partial: bool,
}
const INDENT: &str = "    ";
// Large types keep declarations once their bodies exceed this budget.
const BODY_BUDGET: usize = 2 * 1024 * 1024;
#[derive(Default)]
struct Writer {
    text: String,
    units: usize,
    references: Vec<SourceReference>,
}
impl Writer {
    fn push(&mut self, s: &str) {
        self.units += s.encode_utf16().count();
        self.text.push_str(s);
    }
    fn lines(&mut self, depth: usize, s: &str) {
        self.block(depth, s, &[]);
    }
    // Indents each line and moves body-relative references to their new offsets.
    fn block(&mut self, depth: usize, s: &str, references: &[SourceReference]) {
        let s = s.trim_end_matches('\n');
        let pad = INDENT.repeat(depth);
        let pad_units = pad.encode_utf16().count();
        let mut offset = 0;
        for line in s.split('\n') {
            let len = line.encode_utf16().count();
            let start = self.units + if line.is_empty() { 0 } else { pad_units };
            for r in references
                .iter()
                .filter(|r| r.start >= offset && r.end <= offset + len)
            {
                self.references.push(SourceReference {
                    start: start + r.start - offset,
                    end: start + r.end - offset,
                    token: r.token,
                });
            }
            if !line.is_empty() {
                self.push(&pad);
                self.push(line);
            }
            self.push("\n");
            offset += len + 1;
        }
    }
}
/// dnSpy's `// Token: … RID: … RVA: … File Offset: …` member header.
pub fn token_comment(a: &Assembly, label: &str, token: u32) -> String {
    let mut text = format!("// {label}Token: 0x{token:08X} RID: {}", token & 0xffffff);
    if token >> 24 == 6
        && let Ok(row) = a.method_row(token)
        && row.rva != 0
    {
        text.push_str(&format!(" RVA: 0x{:08X}", row.rva));
        if let Ok(range) = a.image.range(&a.bytes, row.rva, 1) {
            text.push_str(&format!(" File Offset: 0x{:08X}", range.start));
        }
    }
    text
}
// The compiler's auto-property accessor shapes; returns the backing field they use.
pub(crate) fn backing_field(a: &Assembly, accessor: u32, setter: bool) -> Option<u32> {
    let body = a.body(accessor).ok()??;
    if !body.exceptions.is_empty() {
        return None;
    }
    let code: Vec<_> = body.instructions.iter().filter(|i| i.opcode != 0).collect();
    let field = |i: usize| match code.get(i).map(|i| &i.operand) {
        Some(Operand::Token(t)) if t >> 24 == 4 => Some(*t),
        _ => None,
    };
    let ops: Vec<u16> = code.iter().map(|i| i.opcode).collect();
    match (setter, ops.as_slice()) {
        (false, [0x02, 0x7b, 0x2a]) => field(1),
        (false, [0x7e, 0x2a]) => field(0),
        (true, [0x02, 0x03, 0x7d, 0x2a]) => field(2),
        (true, [0x02, 0x80, 0x2a]) => field(1),
        _ => None,
    }
}
// The backing field of an auto-property or field-like event, when the member qualifies.
fn compact_backing_field(a: &Assembly, source: &MemberSource) -> Option<u32> {
    let owner = a.owners.get(&source.token)?;
    let field = |name: &str| {
        a.nodes
            .iter()
            .find(|n| n.token >> 24 == 4 && n.parent == *owner && n.name == name)
            .map(|n| n.token)
    };
    match source.token >> 24 {
        23 => {
            let expected = field(&format!("<{}>k__BackingField", source.name))?;
            source
                .accessors
                .iter()
                .all(|(kind, t)| {
                    backing_field(a, *t, kind.ends_with("set") || kind.ends_with("init"))
                        == Some(expected)
                })
                .then_some(expected)
        }
        20 => {
            let field = field(&source.name)?;
            source
                .accessors
                .iter()
                .all(|(_, t)| attributes::compiler_generated(a, *t))
                .then_some(field)
        }
        _ => None,
    }
}
fn attribute_lines(w: &mut Writer, a: &Assembly, depth: usize, token: u32) {
    for line in attributes::render(a, token) {
        w.lines(depth, &line);
    }
}
// Conventional C# member order: fields, constructors, properties, events, methods.
fn rank(source: &MemberSource) -> u8 {
    match (source.token >> 24, source.kind.as_str()) {
        (4, _) => 0,
        (6, "constructor") => 1,
        (23, _) => 2,
        (20, _) => 3,
        _ => 4,
    }
}
struct Render<'a> {
    a: &'a Assembly,
    accessor: &'a dyn Fn(u32) -> Option<Accessor>,
    types: Vec<TypeDeclaration>,
    out: Writer,
    partial: bool,
    body_bytes: usize,
}
impl Render<'_> {
    fn source(&mut self, token: u32) -> Option<MemberSource> {
        if self.body_bytes > BODY_BUDGET {
            self.partial = true;
            return None;
        }
        let source = export::member_with(self.a, token, self.accessor).ok()?;
        self.body_bytes += source.body.as_ref().map_or(0, String::len);
        Some(source)
    }
    fn method(&mut self, depth: usize, source: &MemberSource, label: &str) {
        self.out
            .lines(depth, &token_comment(self.a, label, source.token));
        attribute_lines(&mut self.out, self.a, depth, source.token);
        match (&source.body, source.quality.as_str()) {
            (Some(body), "csharp_like") => {
                self.out.lines(depth, &source.declaration);
                self.out.block(depth, body, &source.references);
            }
            (_, "annotated_il" | "error") => {
                self.partial = true;
                self.out.lines(depth, &format!("// {}", source.declaration));
                let reason = source
                    .diagnostics
                    .last()
                    .map(|d| d.replace(['\n', '\r'], " "))
                    .unwrap_or_default();
                self.out.lines(
                    depth,
                    &format!("// C# reconstruction is unavailable; select the member to read its IL. {reason}"),
                );
            }
            _ => self.out.lines(depth, &format!("{};", source.declaration)),
        }
    }
    fn accessor_body(&mut self, depth: usize, kind: &str, token: u32) {
        let Some(source) = self.source(token) else {
            self.out.lines(depth, &format!("{kind};"));
            return;
        };
        let label = kind.rsplit(' ').next().unwrap_or(kind);
        self.out
            .lines(depth, &token_comment(self.a, &format!("({label}) "), token));
        match (&source.body, source.quality.as_str()) {
            (Some(body), "csharp_like") => {
                self.out.lines(depth, kind);
                self.out.block(depth, body, &source.references);
            }
            (_, "annotated_il" | "error") => {
                self.partial = true;
                self.out.lines(depth, &format!("{kind};"));
                self.out
                    .lines(depth, "// Accessor body is available in the IL view.");
            }
            _ => self.out.lines(depth, &format!("{kind};")),
        }
    }
    fn member(&mut self, depth: usize, source: &MemberSource, compact: bool) {
        match source.token >> 24 {
            6 => self.method(depth, source, ""),
            23 | 20 if compact => {
                self.out
                    .lines(depth, &token_comment(self.a, "", source.token));
                for (kind, token) in &source.accessors {
                    let kind = kind.rsplit(' ').next().unwrap_or(kind);
                    self.out
                        .lines(depth, &token_comment(self.a, &format!("({kind}) "), *token));
                }
                attribute_lines(&mut self.out, self.a, depth, source.token);
                let line = if source.token >> 24 == 20 {
                    format!("{};", source.declaration)
                } else {
                    // An accessor narrower than the property keeps its own modifier.
                    let parts: Vec<String> = source
                        .accessors
                        .iter()
                        .map(|(kind, token)| {
                            let kind = kind.rsplit(' ').next().unwrap_or(kind);
                            let access = self
                                .a
                                .method_row(*token)
                                .map(|row| visibility(row.flags as u32))
                                .unwrap_or("");
                            if access.is_empty()
                                || source.declaration.starts_with(&format!("{access} "))
                            {
                                format!("{kind};")
                            } else {
                                format!("{access} {kind};")
                            }
                        })
                        .collect();
                    format!("{} {{ {} }}", source.declaration, parts.join(" "))
                };
                self.out.lines(depth, &line);
            }
            23 | 20 => {
                self.out
                    .lines(depth, &token_comment(self.a, "", source.token));
                attribute_lines(&mut self.out, self.a, depth, source.token);
                self.out.lines(depth, &source.declaration);
                self.out.lines(depth, "{");
                for (kind, token) in &source.accessors {
                    self.accessor_body(depth + 1, kind, *token);
                }
                self.out.lines(depth, "}");
            }
            _ => {
                self.out
                    .lines(depth, &token_comment(self.a, "", source.token));
                attribute_lines(&mut self.out, self.a, depth, source.token);
                self.out.lines(depth, &source.declaration);
            }
        }
    }
    fn render(&mut self, index: usize, depth: usize) -> Result<()> {
        if depth > 48 {
            return Err(Error::limit("Nested type view exceeds 48 levels"));
        }
        let (token, kind, declaration, members) = {
            let t = &self.types[index];
            (
                t.token,
                t.kind.clone(),
                t.declaration.clone(),
                t.members.clone(),
            )
        };
        self.out.lines(depth, &token_comment(self.a, "", token));
        attribute_lines(&mut self.out, self.a, depth, token);
        if kind == "delegate" {
            self.out.lines(depth, &declaration);
            return Ok(());
        }
        let header = if kind == "module" {
            "internal class <Module>".to_owned()
        } else {
            declaration
        };
        // A record whose members are all synthesized is a single declaration.
        if members.is_empty()
            && header.contains(" record ")
            && !self.types.iter().any(|t| t.parent == token)
        {
            self.out.lines(depth, &format!("{header};"));
            return Ok(());
        }
        self.out.lines(depth, &header);
        self.out.lines(depth, "{");
        let mut sources: Vec<MemberSource> =
            members.iter().filter_map(|t| self.source(*t)).collect();
        if sources.len() < members.len() {
            self.partial = true;
        }
        let accessors: HashSet<u32> = sources
            .iter()
            .flat_map(|s| s.accessors.iter().map(|(_, t)| *t))
            .collect();
        sources.retain(|s| !accessors.contains(&s.token) && s.quality != "implicit");
        // Auto-properties and field-like events are written compactly and their
        // compiler-generated backing fields are hidden, as dnSpy/ILSpy show them.
        let mut compact = HashSet::new();
        let mut hidden = HashSet::new();
        for source in &sources {
            if let Some(field) = compact_backing_field(self.a, source) {
                compact.insert(source.token);
                hidden.insert(field);
            }
        }
        sources.retain(|s| !hidden.contains(&s.token));
        sources.sort_by_key(rank);
        let mut first = true;
        for source in &sources {
            if !first {
                self.out.push("\n");
            }
            first = false;
            self.member(depth + 1, source, compact.contains(&source.token));
        }
        let nested: Vec<usize> = (0..self.types.len())
            .filter(|i| self.types[*i].parent == token && self.types[*i].token != token)
            .collect();
        for child in nested {
            if !first {
                self.out.push("\n");
            }
            first = false;
            self.render(child, depth + 1)?;
        }
        self.out.lines(depth, "}");
        Ok(())
    }
}
pub fn type_source(
    a: &Assembly,
    token: u32,
    accessor: &dyn Fn(u32) -> Option<Accessor>,
) -> Result<TypeSource> {
    a.validate_token(token)?;
    if token >> 24 != 2 {
        return Err(Error::metadata("A type view requires a TypeDef token"));
    }
    // Nested types live in their outermost type's namespace.
    let mut outer = token;
    for _ in 0..49 {
        match a.owners.get(&outer) {
            Some(parent) if *parent >> 24 == 2 => outer = *parent,
            _ => break,
        }
    }
    let namespace = a
        .nodes
        .iter()
        .find(|n| n.token == outer)
        .map(|n| n.namespace.clone())
        .unwrap_or_default();
    let scope = a.short_names(&namespace);
    let types = export::plan(a, Some(token))?;
    let root = types
        .iter()
        .position(|t| t.token == token)
        .ok_or_else(|| Error::metadata("Type is missing from its own plan"))?;
    let mut render = Render {
        a,
        accessor,
        types,
        out: Writer::default(),
        partial: false,
        body_bytes: 0,
    };
    let depth = usize::from(!namespace.is_empty());
    render.render(root, depth)?;
    let usings = scope.usings();
    drop(scope);
    let mut out = Writer::default();
    out.lines(
        0,
        "// Reconstructed from CIL and metadata; local names are synthetic.",
    );
    for namespace in &usings {
        out.lines(0, &format!("using {namespace};"));
    }
    out.push("\n");
    if !namespace.is_empty() {
        out.lines(0, &format!("namespace {namespace}"));
        out.lines(0, "{");
    }
    let offset = out.units;
    out.push(&render.out.text);
    out.references
        .extend(render.out.references.into_iter().map(|r| SourceReference {
            start: r.start + offset,
            end: r.end + offset,
            token: r.token,
        }));
    if !namespace.is_empty() {
        out.lines(0, "}");
    }
    if render.body_bytes > BODY_BUDGET {
        out.lines(
            0,
            "// Type view size limit reached; remaining bodies are available per member.",
        );
    }
    Ok(TypeSource {
        text: out.text,
        references: out.references,
        partial: render.partial,
    })
}
/// The dnSpy member view: one field, property or event with its token comments.
pub fn member_source(
    a: &Assembly,
    token: u32,
    accessor: &dyn Fn(u32) -> Option<Accessor>,
) -> Result<TypeSource> {
    a.validate_token(token)?;
    let mut owner = *a.owners.get(&token).unwrap_or(&0);
    while let Some(parent) = a.owners.get(&owner).filter(|p| **p >> 24 == 2) {
        owner = *parent;
    }
    let namespace = a
        .nodes
        .iter()
        .find(|n| n.token == owner)
        .map(|n| n.namespace.clone())
        .unwrap_or_default();
    let _scope = a.short_names(&namespace);
    let source = export::member_with(a, token, accessor)?;
    let mut render = Render {
        a,
        accessor,
        types: vec![],
        out: Writer::default(),
        partial: false,
        body_bytes: 0,
    };
    let compact = compact_backing_field(a, &source).is_some();
    render.member(0, &source, compact);
    Ok(TypeSource {
        text: render.out.text,
        references: render.out.references,
        partial: render.partial,
    })
}
