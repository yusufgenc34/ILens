//! Editing a method in C#, as dnSpy does. The method's outermost type becomes a
//! compilable skeleton in which every other member is a stub; the compiler (Roslyn,
//! loaded separately in the browser) compiles it against the assembly and its loaded
//! dependencies as metadata. `merge` then maps the compiled body's tokens back into
//! the edited assembly: its own definitions by identity, other assemblies' references
//! to existing rows, and anything missing to new TypeRef/MemberRef/TypeSpec/MethodSpec/
//! AssemblyRef rows. The merged IL goes through the same write verifier as IL edits.
use crate::{
    assembly::Assembly,
    cil_encode::EditableInstruction,
    edit::EditRegion,
    error::{Error, ErrorCode, Result},
    metadata_write::Tables,
    signature::{self, Type},
};
use serde::Serialize;
use std::collections::HashMap;

fn unsupported(detail: impl Into<String>) -> Error {
    Error::new(ErrorCode::UnsupportedEdit, detail)
}

/// The editable method and the skeleton around it; `before + method + after`
/// compiles.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub method: String,
    pub before: String,
    pub after: String,
}

fn indent(text: &str, depth: usize) -> String {
    let pad = "    ".repeat(depth);
    text.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The member's text inside the skeleton: declarations without bodies.
fn stub(a: &Assembly, token: u32, interface: bool) -> Result<Option<String>> {
    let member = crate::export::member(a, token)?;
    let declaration = member.declaration.trim().to_owned();
    let abstract_ = interface || declaration.contains(" abstract ");
    Ok(Some(match token >> 24 {
        4 => {
            if declaration.ends_with(';') {
                declaration
            } else {
                format!("{declaration};")
            }
        }
        6 => {
            let node = a.nodes.iter().find(|n| n.token == token);
            let name = node.map(|n| n.name.as_str()).unwrap_or("");
            if name == ".cctor" {
                return Ok(None);
            }
            if name == ".ctor" {
                // Constructors need no base call when they are extern.
                let owner = a.method_ref(token)?.owner;
                let simple = crate::assembly::strip_arity(
                    &a.nodes
                        .iter()
                        .find(|n| n.token == owner)
                        .map(|n| n.name.clone())
                        .unwrap_or_default(),
                );
                let at = declaration
                    .find(&format!("{simple}("))
                    .ok_or_else(|| unsupported("Unrecognized constructor declaration"))?;
                format!("{}extern {};", &declaration[..at], &declaration[at..])
            } else if abstract_ || declaration.contains(" extern ") {
                format!("{declaration};")
            } else {
                format!("{declaration} => throw null;")
            }
        }
        23 => {
            let accessors: Vec<String> = member
                .accessors
                .iter()
                .map(|(kind, _)| {
                    if abstract_ {
                        format!("{kind};")
                    } else {
                        format!("{kind} => throw null;")
                    }
                })
                .collect();
            format!("{declaration} {{ {} }}", accessors.join(" "))
        }
        20 => {
            if abstract_ {
                format!("{declaration};")
            } else {
                format!("{declaration} {{ add {{ }} remove {{ }} }}")
            }
        }
        _ => return Ok(None),
    }))
}

/// The outermost type declaring `method`.
fn top_type(a: &Assembly, method: u32) -> Result<u32> {
    let mut owner = a.method_ref(method)?.owner;
    for _ in 0..64 {
        let node = a
            .nodes
            .iter()
            .find(|n| n.token == owner)
            .ok_or_else(|| unsupported("The declaring type is missing"))?;
        if node.parent >> 24 != 2 {
            return Ok(owner);
        }
        owner = node.parent;
    }
    Err(unsupported("Type nesting is too deep"))
}

pub fn source(a: &Assembly, method: u32) -> Result<Source> {
    if method >> 24 != 6 {
        return Err(unsupported("Select a method to edit in C#"));
    }
    let top = top_type(a, method)?;
    if crate::records::record(a, top).is_some() {
        return Err(unsupported(
            "Methods of records cannot be edited in C# yet; edit their IL instead",
        ));
    }
    let plan = crate::export::plan(a, Some(top))?;
    let edited = crate::export::member(a, method)?;
    let body = match (&edited.body, edited.quality.as_str()) {
        (Some(body), "csharp_like") => body.clone(),
        _ => "{\n    // C# reconstruction is unavailable for this method; write its body.\n    throw new System.NotImplementedException();\n}".to_owned(),
    };
    let method_text = format!("{}\n{}", edited.declaration.trim(), body.trim_end());
    const MARKER: &str = "\u{0}METHOD\u{0}";
    // Types render recursively: members, then nested types.
    fn render(
        a: &Assembly,
        plan: &[crate::export::TypeDeclaration],
        index: usize,
        method: u32,
        depth: usize,
        out: &mut String,
    ) -> Result<()> {
        let t = &plan[index];
        let interface = t.kind == "interface";
        out.push_str(&indent(&t.declaration, depth));
        out.push('\n');
        out.push_str(&indent("{", depth));
        out.push('\n');
        let mut accessors = std::collections::HashSet::new();
        for m in &t.members {
            if matches!(m >> 24, 20 | 23)
                && let Ok(source) = crate::export::member(a, *m)
            {
                accessors.extend(source.accessors.iter().map(|(_, token)| *token));
            }
        }
        for m in &t.members {
            // Accessors belong to their property; compiler-generated members (backing
            // fields) are not part of the source.
            let generated = a
                .nodes
                .iter()
                .any(|n| n.token == *m && n.name.starts_with('<'));
            if accessors.contains(m) || generated {
                continue;
            }
            if *m == method {
                out.push_str(MARKER);
                out.push('\n');
                continue;
            }
            if let Some(text) = stub(a, *m, interface)? {
                out.push_str(&indent(&text, depth + 1));
                out.push('\n');
            }
        }
        for (i, nested) in plan.iter().enumerate() {
            if nested.parent == t.token && i != index {
                render(a, plan, i, method, depth + 1, out)?;
            }
        }
        out.push_str(&indent("}", depth));
        out.push('\n');
        Ok(())
    }
    let root = plan
        .iter()
        .position(|t| t.token == top)
        .ok_or_else(|| unsupported("The declaring type cannot be exported"))?;
    let namespace = &plan[root].namespace;
    let mut text = String::from("#pragma warning disable\n");
    let depth = usize::from(!namespace.is_empty());
    if !namespace.is_empty() {
        text.push_str(&format!("namespace {namespace}\n{{\n"));
    }
    render(a, &plan, root, method, depth, &mut text)?;
    if !namespace.is_empty() {
        text.push_str("}\n");
    }
    let at = text
        .find(MARKER)
        .ok_or_else(|| unsupported("The method is not part of its type's export"))?;
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    Ok(Source {
        method: method_text,
        before: text[..line_start].to_owned(),
        after: text[at + MARKER.len()..].to_owned(),
    })
}

/// A row the merged body needs that the edited assembly does not have yet. Tokens are
/// assigned in order after the existing rows of each table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewRow {
    AssemblyRef {
        version: [u16; 4],
        flags: u32,
        public_key: Vec<u8>,
        name: String,
        culture: String,
    },
    TypeRef {
        scope: u32,
        name: String,
        namespace: String,
    },
    MemberRef {
        parent: u32,
        name: String,
        signature: Vec<u8>,
    },
    TypeSpec {
        signature: Vec<u8>,
    },
    MethodSpec {
        method: u32,
        instantiation: Vec<u8>,
    },
}
impl NewRow {
    pub fn table(&self) -> usize {
        match self {
            NewRow::AssemblyRef { .. } => 35,
            NewRow::TypeRef { .. } => 1,
            NewRow::MemberRef { .. } => 10,
            NewRow::TypeSpec { .. } => 27,
            NewRow::MethodSpec { .. } => 43,
        }
    }
}

/// The compiled method mapped into the edited assembly.
#[derive(Debug, Clone)]
pub struct Merged {
    pub instructions: Vec<EditableInstruction>,
    pub locals: Vec<Type>,
    pub regions: Vec<EditRegion>,
    pub rows: Vec<NewRow>,
}

fn stream<'a>(a: &'a Assembly, name: &str) -> &'a [u8] {
    let metadata = &a.bytes[a.image.metadata.clone()];
    a.store
        .streams
        .iter()
        .find(|s| s.name == name)
        .map_or(&[][..], |s| &metadata[s.offset..s.offset + s.size])
}

struct Mapper<'a> {
    a: &'a Assembly,
    c: &'a Assembly,
    a_tables: Tables,
    c_tables: Tables,
    rows: Vec<NewRow>,
    cache: HashMap<u32, u32>,
}
impl Mapper<'_> {
    fn string(asm: &Assembly, offset: u32) -> String {
        asm.string(offset).map(str::to_owned).unwrap_or_default()
    }
    fn next(&self, table: usize) -> u32 {
        let existing = self.a_tables.rows[table].len() as u32;
        let added = self.rows.iter().filter(|r| r.table() == table).count() as u32;
        ((table as u32) << 24) | (existing + added + 1)
    }
    fn add(&mut self, row: NewRow) -> u32 {
        if let Some(i) = self.rows.iter().position(|r| *r == row) {
            let table = row.table();
            let before = self.rows[..i].iter().filter(|r| r.table() == table).count() as u32;
            return ((table as u32) << 24) | (self.a_tables.rows[table].len() as u32 + before + 1);
        }
        let token = self.next(row.table());
        self.rows.push(row);
        token
    }
    /// Whether a compiled-assembly AssemblyRef names the edited assembly itself.
    fn is_self(&self, assembly_ref: u32) -> bool {
        let row = &self.c_tables.rows[35][(assembly_ref & 0xff_ffff) as usize - 1];
        Self::string(self.c, row[6]) == self.a.info.identity.name
    }
    fn map(&mut self, token: u32) -> Result<u32> {
        if let Some(t) = self.cache.get(&token) {
            return Ok(*t);
        }
        let mapped = self.map_uncached(token)?;
        self.cache.insert(token, mapped);
        Ok(mapped)
    }
    /// The edited assembly's definition with this full name.
    fn definition(&self, full_name: &str) -> Result<u32> {
        self.a
            .nodes
            .iter()
            .find(|n| n.token >> 24 == 2 && n.full_name == full_name)
            .map(|n| n.token)
            .ok_or_else(|| {
                unsupported(format!(
                    "The C# code needs a new type ({full_name}): lambdas, local functions, iterators and async methods are not supported yet"
                ))
            })
    }
    fn member_definition(&self, owner: u32, name: &str, c_token: u32, field: bool) -> Result<u32> {
        self.a
            .nodes
            .iter()
            .filter(|n| n.parent == owner && n.name == name && (n.token >> 24 == 4) == field && matches!(n.token >> 24, 4 | 6))
            .find(|n| crate::resolution::same_member(self.c, c_token, self.a, n.token, field))
            .map(|n| n.token)
            .ok_or_else(|| {
                unsupported(format!(
                    "{}.{name} does not exist in the edited assembly; adding members from C# is not supported yet",
                    self.a.resolve_full(owner)
                ))
            })
    }
    fn map_uncached(&mut self, token: u32) -> Result<u32> {
        let row = (token & 0xff_ffff) as usize;
        match token >> 24 {
            // Types and members of the compiled skeleton are the edited assembly's own.
            2 => self.definition(&self.c.resolve_full(token)),
            4 | 6 => {
                let owner_c = if token >> 24 == 4 {
                    self.c.field_ref(token)?.0
                } else {
                    self.c.method_ref(token)?.owner
                };
                let owner = self.definition(&self.c.resolve_full(owner_c))?;
                let name = self
                    .c
                    .nodes
                    .iter()
                    .find(|n| n.token == token)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                self.member_definition(owner, &name, token, token >> 24 == 4)
            }
            1 => {
                let r = self.c_tables.rows[1][row - 1].clone();
                let scope = coded_value(1, r[0]);
                if scope >> 24 == 35 && self.is_self(scope) {
                    return self.definition(&self.c.resolve_full(token));
                }
                if scope >> 24 == 1 {
                    let outer = self.map(scope)?;
                    if outer >> 24 == 2 {
                        return self.definition(&self.c.resolve_full(token));
                    }
                }
                let new_scope = match scope >> 24 {
                    35 => self.map_assembly(scope)?,
                    1 => self.map(scope)?,
                    _ => return Err(unsupported("Unsupported type reference scope")),
                };
                let (name, namespace) = (Self::string(self.c, r[1]), Self::string(self.c, r[2]));
                let existing = self.a_tables.rows[1].iter().position(|x| {
                    coded_value(1, x[0]) == new_scope
                        && Self::string(self.a, x[1]) == name
                        && Self::string(self.a, x[2]) == namespace
                });
                Ok(match existing {
                    Some(i) => 0x0100_0001 + i as u32,
                    None => self.add(NewRow::TypeRef {
                        scope: new_scope,
                        name,
                        namespace,
                    }),
                })
            }
            27 => {
                let r = self.c_tables.rows[27][row - 1].clone();
                let ty = signature::type_spec(self.c.blob(r[0])?)?;
                let mapped = signature::map_tokens(&ty, &mut |t| self.map(t))?;
                let existing = self.a_tables.rows[27].iter().position(|x| {
                    self.a
                        .blob(x[0])
                        .ok()
                        .and_then(|b| signature::type_spec(b).ok())
                        .is_some_and(|t| t == mapped)
                });
                Ok(match existing {
                    Some(i) => 0x1b00_0001 + i as u32,
                    None => {
                        let mut signature = vec![];
                        signature::encode_type(&mapped, &mut signature)?;
                        self.add(NewRow::TypeSpec { signature })
                    }
                })
            }
            10 => {
                let r = self.c_tables.rows[10][row - 1].clone();
                let parent = self.map(coded_value(10, r[0]))?;
                let name = Self::string(self.c, r[1]);
                let blob = self.c.blob(r[2])?.to_vec();
                let field = blob.first() == Some(&6);
                // A member of the edited assembly's own non-generic types is a definition.
                if parent >> 24 == 2 {
                    return self.member_definition(parent, &name, token, field);
                }
                let signature = if field {
                    signature::encode_field(&signature::map_tokens(
                        &signature::field(&blob)?,
                        &mut |t| self.map(t),
                    )?)?
                } else {
                    signature::encode_method(&signature::map_method_tokens(
                        &signature::method(&blob)?,
                        &mut |t| self.map(t),
                    )?)?
                };
                let existing = self.a_tables.rows[10].iter().position(|x| {
                    coded_value(10, x[0]) == parent
                        && Self::string(self.a, x[1]) == name
                        && self.a.blob(x[2]).is_ok_and(|b| b == signature.as_slice())
                });
                Ok(match existing {
                    Some(i) => 0x0a00_0001 + i as u32,
                    None => self.add(NewRow::MemberRef {
                        parent,
                        name,
                        signature,
                    }),
                })
            }
            43 => {
                let r = self.c_tables.rows[43][row - 1].clone();
                let method = self.map(coded_value(43, r[0]))?;
                let args = signature::method_spec(self.c.blob(r[1])?)?
                    .iter()
                    .map(|t| signature::map_tokens(t, &mut |x| self.map(x)))
                    .collect::<Result<Vec<_>>>()?;
                let instantiation = signature::encode_method_spec(&args)?;
                let existing = self.a_tables.rows[43].iter().position(|x| {
                    coded_value(43, x[0]) == method
                        && self
                            .a
                            .blob(x[1])
                            .is_ok_and(|b| b == instantiation.as_slice())
                });
                Ok(match existing {
                    Some(i) => 0x2b00_0001 + i as u32,
                    None => self.add(NewRow::MethodSpec {
                        method,
                        instantiation,
                    }),
                })
            }
            _ => Err(unsupported(format!(
                "The compiled code uses a 0x{:02X} token that cannot be merged",
                token >> 24
            ))),
        }
    }
    fn map_assembly(&mut self, token: u32) -> Result<u32> {
        let r = self.c_tables.rows[35][(token & 0xff_ffff) as usize - 1].clone();
        let name = Self::string(self.c, r[6]);
        if let Some(i) = self.a_tables.rows[35]
            .iter()
            .position(|x| Self::string(self.a, x[6]) == name)
        {
            return Ok(0x2300_0001 + i as u32);
        }
        Ok(self.add(NewRow::AssemblyRef {
            version: [r[0] as u16, r[1] as u16, r[2] as u16, r[3] as u16],
            flags: r[4],
            public_key: self.c.blob(r[5])?.to_vec(),
            name,
            culture: Self::string(self.c, r[7]),
        }))
    }
}

/// A coded index column value in table `table`'s first coded column, as a token.
fn coded_value(table: usize, value: u32) -> u32 {
    use crate::metadata::Column;
    let column = crate::metadata::schema(table)
        .into_iter()
        .find_map(|c| match c {
            Column::Coded(k) => Some(k),
            _ => None,
        })
        .expect("table has a coded column");
    let bits = column.tag_bits();
    match crate::metadata::coded_tables(column).get((value & ((1 << bits) - 1)) as usize) {
        Some(Some(t)) => ((*t as u32) << 24) | (value >> bits),
        _ => 0,
    }
}

/// Maps the compiled skeleton's version of `method` into the edited assembly.
pub fn merge(a: &Assembly, method: u32, compiled: &[u8]) -> Result<Merged> {
    let c = Assembly::load(compiled.to_vec())?;
    let owner = a.method_ref(method)?.owner;
    let owner_name = a.resolve_full(owner);
    let name = a
        .nodes
        .iter()
        .find(|n| n.token == method)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let c_owner = c
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 2 && n.full_name == owner_name)
        .ok_or_else(|| unsupported("The compiled code lost the declaring type"))?
        .token;
    let compiled_method = c
        .nodes
        .iter()
        .filter(|n| n.parent == c_owner && n.token >> 24 == 6 && n.name == name)
        .find(|n| crate::resolution::same_member(&c, n.token, a, method, false))
        .ok_or_else(|| {
            unsupported("The method's signature changed; change only its body when editing in C#")
        })?
        .token;
    // Generated helper types mean lambdas, iterators or async code.
    if let Some(helper) = c
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 2 && n.name.starts_with('<') && n.name != "<Module>")
    {
        return Err(unsupported(format!(
            "The C# code needs a generated type ({}): lambdas, local functions, iterators and async methods are not supported yet",
            helper.name
        )));
    }
    let body = c
        .body(compiled_method)?
        .ok_or_else(|| unsupported("The compiled method has no body"))?;
    let mut mapper = Mapper {
        a,
        c: &c,
        a_tables: Tables::decode(stream(a, "#~"))?,
        c_tables: Tables::decode(stream(&c, "#~"))?,
        rows: vec![],
        cache: HashMap::new(),
    };
    let mut instructions = crate::cil_encode::from_body(&body);
    for (row, i) in instructions.iter_mut().zip(&body.instructions) {
        if let crate::cil::Operand::Token(t) = i.operand {
            row.operand = if t >> 24 == 0x70 {
                serde_json::to_string(&c.user_string(t)?).unwrap_or_default()
            } else {
                format!("0x{:08X}", mapper.map(t)?)
            };
        }
    }
    let locals = c
        .locals(&body)?
        .iter()
        .map(|t| signature::map_tokens(t, &mut |x| mapper.map(x)))
        .collect::<Result<Vec<_>>>()?;
    let label = |offset: u32| {
        if offset >= body.code_size {
            String::new()
        } else {
            format!("IL_{offset:04X}")
        }
    };
    let mut regions = vec![];
    for e in &body.exceptions {
        regions.push(EditRegion {
            kind: e.kind.clone(),
            try_start: label(e.try_start),
            try_end: label(e.try_end),
            handler_start: label(e.handler_start),
            handler_end: label(e.handler_end),
            catch_type: match e.catch_type {
                Some(t) => Some(format!("0x{:08X}", mapper.map(t)?)),
                None => None,
            },
        });
    }
    Ok(Merged {
        instructions,
        locals,
        regions,
        rows: mapper.rows,
    })
}
