//! Static reader for one legacy Eazfuscator-compatible resource-string layout.
//! Independent format implementation: no de4dot code/runtime, reflection, cctor,
//! or assembly execution. A closed, fingerprinted layout supplies expression
//! anchors; constants, tokens, resource names and keys come from the input.
use crate::{
    assembly::Assembly,
    cil::{Instruction, MethodBody, Operand},
    error::{Error, Result},
    signature::Type,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_TEXT: usize = 1024 * 1024;
const MAX_TOTAL: usize = 8 * 1024 * 1024;
fn stop(s: &str) -> Error {
    crate::edit::unsupported(format!("Legacy resource strings: {s}"))
}
fn body(a: &Assembly, token: u32) -> Result<MethodBody> {
    a.body(token)?.ok_or_else(|| stop("missing helper body"))
}
fn at(b: &MethodBody, offset: u32) -> Result<&Instruction> {
    b.instructions
        .iter()
        .find(|i| i.offset == offset)
        .ok_or_else(|| stop("unrecognized helper layout"))
}
fn token_at(b: &MethodBody, offset: u32) -> Result<u32> {
    at(b, offset)?
        .token()
        .ok_or_else(|| stop("expected metadata operand"))
}

/// Structural fingerprints are dispatch filters, not authenticity signatures.
/// Local/branch operands and small constants are fixed. Full-width key literals
/// and metadata row numbers vary; all values are subsequently bounds checked.
fn shape(b: &MethodBody, fixed: bool) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for i in &b.instructions {
        let value = match i.operand {
            Operand::Integer(n) if fixed || !matches!(i.opcode, 0x20 | 0x21) => n as u64,
            Operand::Variable(n) => n as u64,
            Operand::Branch(n) => n as u64,
            Operand::Token(t) => (t >> 24) as u64,
            _ => 0,
        };
        for n in [
            i.offset,
            i.opcode as u32,
            value as u32,
            (value >> 32) as u32,
        ] {
            for byte in n.to_le_bytes() {
                h = (h ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    }
    h
}
fn expect_shape(b: &MethodBody, expected: u64, fixed: bool) -> Result<()> {
    if shape(b, fixed) != expected {
        return Err(stop("unsupported helper variant; no runtime fallback"));
    }
    Ok(())
}
// Select modeled core-library semantics; assembly names are not authentication.
fn core_type(a: &Assembly, token: u32, name: &str) -> bool {
    if token >> 24 != 1 || a.resolve_full(token) != name {
        return false;
    }
    let Some(row) = a
        .metadata()
        .type_refs
        .get((token & 0xffffff).saturating_sub(1) as usize)
    else {
        return false;
    };
    let scope = crate::assembly::coded(row.resolution_scope);
    a.references.iter().any(|r| {
        r.token == scope
            && r.identity.culture == "neutral"
            && matches!(
                (
                    r.identity.name.as_str(),
                    r.identity.public_key_token.as_str()
                ),
                ("mscorlib", "b77a5c561934e089")
                    | ("System.Runtime", "b03f5f7f11d50a3a")
                    | ("System.Private.CoreLib", "7cec85d7bea7798e")
                    | ("netstandard", "cc7b13ffcd2ddd51")
            )
    })
}
#[derive(Clone, Copy)]
enum Scalar {
    Int(i32),
    Handle(u32),
    Type(u32),
}
impl Scalar {
    fn int(self) -> Result<i32> {
        if let Self::Int(x) = self {
            Ok(x)
        } else {
            Err(stop("non-integer key expression"))
        }
    }
}
struct Expressions<'a> {
    a: &'a Assembly,
    steps: usize,
}
struct State {
    stack: Vec<Scalar>,
    locals: BTreeMap<usize, Scalar>,
    chars: Vec<u16>,
}
impl State {
    fn new() -> Self {
        Self {
            stack: vec![],
            locals: BTreeMap::new(),
            chars: vec![],
        }
    }
    fn pop(&mut self) -> Result<Scalar> {
        self.stack
            .pop()
            .ok_or_else(|| stop("key-expression stack underflow"))
    }
    fn integer(&mut self) -> Result<i32> {
        self.pop()?.int()
    }
}
impl Expressions<'_> {
    fn run(
        &mut self,
        b: &MethodBody,
        range: std::ops::Range<u32>,
        st: &mut State,
        args: &[Scalar],
        depth: usize,
        builder: bool,
    ) -> Result<()> {
        if depth > 16 {
            return Err(Error::limit("Metadata key expression exceeds depth 16"));
        }
        for i in b.instructions.iter().filter(|i| range.contains(&i.offset)) {
            self.steps += 1;
            if self.steps > 20_000 || st.stack.len() > 64 {
                return Err(Error::limit("Metadata key expression budget exhausted"));
            }
            let slot = |base: u16| -> Result<usize> {
                match i.operand {
                    Operand::Variable(n) => Ok(n as usize),
                    _ => Ok((i.opcode - base) as usize),
                }
            };
            if let Some(v) = constant(i) {
                st.stack.push(Scalar::Int(v));
                continue;
            }
            match i.opcode {
                0x00 => {}
                0x02..=0x05 | 0x0e | 0xfe09 => st.stack.push(
                    *args
                        .get(slot(0x02)?)
                        .ok_or_else(|| stop("unknown key argument"))?,
                ),
                0x06..=0x09 | 0x11 | 0xfe0c => st.stack.push(
                    *st.locals
                        .get(&slot(0x06)?)
                        .ok_or_else(|| stop("unknown key local"))?,
                ),
                0x0a..=0x0d | 0x13 | 0xfe0e => {
                    let v = st.pop()?;
                    st.locals.insert(slot(0x0a)?, v);
                }
                0x25 => {
                    let v = st.pop()?;
                    st.stack.extend([v, v]);
                }
                0x26 => {
                    st.pop()?;
                }
                0xd0 => {
                    let t = i.token().ok_or_else(|| stop("missing type token"))?;
                    if t >> 24 != 2 {
                        return Err(stop("external type in key expression"));
                    }
                    self.a.validate_token(t)?;
                    st.stack.push(Scalar::Handle(t));
                }
                0x58..=0x5a | 0x5f..=0x64 => {
                    let y = st.integer()?;
                    let x = st.integer()?;
                    let v = match i.opcode {
                        0x58 => x.wrapping_add(y),
                        0x59 => x.wrapping_sub(y),
                        0x5a => x.wrapping_mul(y),
                        0x5f => x & y,
                        0x60 => x | y,
                        0x61 => x ^ y,
                        0x62 => x.wrapping_shl(y as u32 & 31),
                        0x63 => x.wrapping_shr(y as u32 & 31),
                        0x64 => ((x as u32) >> (y as u32 & 31)) as i32,
                        _ => unreachable!(),
                    };
                    st.stack.push(Scalar::Int(v));
                }
                0x65 | 0x66 | 0x68 | 0x69 | 0x6d | 0xd1 | 0xd2 => {
                    let v = st.integer()?;
                    st.stack.push(Scalar::Int(match i.opcode {
                        0x65 => v.wrapping_neg(),
                        0x66 => !v,
                        0x68 => v as i16 as i32,
                        0xd1 => v as u16 as i32,
                        0xd2 => v as u8 as i32,
                        _ => v,
                    }));
                }
                0x28 | 0x6f => {
                    let t = i.token().ok_or_else(|| stop("missing key call"))?;
                    let m = self.a.method_ref(t)?;
                    if t >> 24 == 0x0a
                        && core_type(self.a, m.owner, "System.Type")
                        && !m.signature.has_this
                        && m.name == "GetTypeFromHandle"
                        && m.signature.parameters.len() == 1
                    {
                        let Scalar::Handle(t) = st.pop()? else {
                            return Err(stop("invalid metadata handle"));
                        };
                        st.stack.push(Scalar::Type(t));
                    } else if t >> 24 == 0x0a
                        && core_type(self.a, m.owner, "System.Reflection.MemberInfo")
                        && m.signature.has_this
                        && m.signature.return_type == Type::primitive("int")
                        && m.name == "get_MetadataToken"
                        && m.signature.parameters.is_empty()
                    {
                        let Scalar::Type(t) = st.pop()? else {
                            return Err(stop("invalid metadata type"));
                        };
                        st.stack.push(Scalar::Int(t as i32));
                    } else if builder
                        && t >> 24 == 0x0a
                        && core_type(self.a, m.owner, "System.Text.StringBuilder")
                        && m.signature.has_this
                        && m.name == "Append"
                        && m.signature.parameters == [Type::primitive("char")]
                    {
                        let ch = st.integer()? as u16;
                        st.chars.push(ch);
                        if st.chars.len() > 256 {
                            return Err(stop("resource name too long"));
                        }
                    } else if t >> 24 == 6
                        && !m.signature.has_this
                        && m.signature.generic_count == 0
                        && m.signature.parameters.len() <= 4
                        && m.signature.return_type == Type::primitive("int")
                    {
                        let mut argv = Vec::new();
                        for _ in &m.signature.parameters {
                            argv.push(st.pop()?);
                        }
                        argv.reverse();
                        let target = body(self.a, t)?;
                        if target.instructions.len() > 256 || !target.exceptions.is_empty() {
                            return Err(stop("non-scalar metadata key helper"));
                        }
                        let mut child = State::new();
                        self.run(&target, 0..u32::MAX, &mut child, &argv, depth + 1, false)?;
                        if child.stack.len() != 1 {
                            return Err(stop("invalid metadata key result"));
                        }
                        st.stack.push(child.pop()?);
                    } else {
                        return Err(stop("external/runtime call in metadata key expression"));
                    }
                }
                0x2a => break,
                _ => return Err(stop("key expression is not a closed integer expression")),
            }
        }
        Ok(())
    }
    fn expr(
        &mut self,
        b: &MethodBody,
        range: std::ops::Range<u32>,
        locals: &BTreeMap<usize, Scalar>,
    ) -> Result<i32> {
        let mut st = State::new();
        st.locals = locals.clone();
        self.run(b, range, &mut st, &[], 0, false)?;
        if st.stack.len() != 1 {
            return Err(stop("ambiguous key expression"));
        }
        st.integer()
    }
}
pub(crate) fn constant(i: &Instruction) -> Option<i32> {
    match (i.opcode, &i.operand) {
        (0x15, _) => Some(-1),
        (0x16..=0x1e, _) => Some(i.opcode as i32 - 0x16),
        (0x1f | 0x20, Operand::Integer(n)) => i32::try_from(*n).ok(),
        _ => None,
    }
}

pub struct Profile {
    pub helper: u32,
    resource: Vec<u8>,
    key_length: i16,
    shared_key: Vec<u8>,
    magics: Vec<i32>,
    magic: Option<i32>,
    offset_xor: i32,
    key_xor: i32,
    flags_xor: i32,
    sentinel: i32,
    redirect: i32,
    latin: i32,
    compressed: i32,
    no_intern: i32,
    length_mask: i32,
    cache: BTreeMap<i32, String>,
    retained: usize,
    work: usize,
}
impl Profile {
    pub fn detect(a: &Assembly) -> Result<Option<Self>> {
        if a.bytes.len() > 16 * 1024 * 1024 || a.metadata().method_defs.len() > 16_384 {
            return Err(Error::limit(
                "Resource-string analysis supports 16 MiB and 16,384 methods",
            ));
        }
        let mut candidate = None;
        let mut scanned = 0usize;
        for (index, row) in a.metadata().method_defs.iter().enumerate() {
            if row.rva == 0 {
                continue;
            }
            let token = 0x06000001 + index as u32;
            let Ok(m) = a.method_ref(token) else {
                continue;
            };
            if m.signature.has_this
                || m.signature.generic_count != 0
                || m.signature.return_type != Type::primitive("string")
                || m.signature.parameters != [Type::primitive("int")]
            {
                continue;
            }
            let Ok(b) = body(a, token) else {
                continue;
            };
            scanned += b.instructions.len();
            if scanned > 2_000_000 {
                return Err(Error::limit(
                    "Resource profile detection instruction budget exceeded",
                ));
            }
            if b.instructions.len() != 894 || shape(&b, false) != 0xc10919bb444a14e8 {
                continue;
            }
            if candidate.is_some() {
                return Err(stop("multiple matching decoders are not supported"));
            }
            candidate = Some((token, b));
        }
        let Some((helper, b)) = candidate else {
            return Ok(None);
        };
        if a.info.identity.public_key_token != "null" {
            return Err(stop("signed profiles are not supported"));
        }
        for (offset, owner, name) in [
            (
                0x13d,
                "System.Reflection.Assembly",
                "GetManifestResourceStream",
            ),
            (0x138, "System.Text.StringBuilder", "ToString"),
            (0x737, "System.String", "Intern"),
        ] {
            let m = a.method_ref(token_at(&b, offset)?)?;
            if !core_type(a, m.owner, owner) || m.name != name {
                return Err(stop("resource reader signature mismatch"));
            }
        }
        expect_shape(&body(a, token_at(&b, 0x5ca)?)?, 0xe863e4e1daa36d2b, true)?;
        expect_shape(&body(a, token_at(&b, 0x662)?)?, 0x3b46babf0583cc14, true)?;
        let hash = body(a, token_at(&b, 0x31a)?)?;
        expect_shape(&hash, 0xf768820316b5e911, false)?;
        let owner = *a
            .owners
            .get(&helper)
            .ok_or_else(|| stop("missing decoder owner"))?;
        let cctors: Vec<_> = a
            .nodes
            .iter()
            .filter(|n| {
                n.token >> 24 == 6 && n.name == ".cctor" && a.owners.get(&n.token) == Some(&owner)
            })
            .collect();
        if cctors.len() != 1 {
            return Err(stop("ambiguous seed definition"));
        }
        let seed_body = body(a, cctors[0].token)?;
        expect_shape(&seed_body, 0xd032ce38b77c9bc1, false)?;
        let mut ex = Expressions { a, steps: 0 };
        let mut st = State::new();
        ex.run(&b, 0..0x11, &mut st, &[], 0, false)?;
        let locals = st.locals.clone();
        st.locals.insert(29, Scalar::Int(0));
        ex.run(&b, 0x6f..0x136, &mut st, &[], 0, true)?;
        let name = String::from_utf16(&st.chars).map_err(|_| stop("invalid resource name"))?;
        let resources: Vec<_> = a
            .metadata()
            .manifest_resources
            .iter()
            .enumerate()
            .filter(|(_, r)| a.string(r.name).is_ok_and(|n| n == name) && r.implementation.row == 0)
            .collect();
        if resources.len() != 1 {
            return Err(stop("missing or ambiguous encrypted resource"));
        }
        let resource = crate::export::resource(a, 0x28000001 + resources[0].0 as u32)?;
        if resource.len() > 8 * 1024 * 1024 {
            return Err(Error::limit("Encrypted string resource exceeds 8 MiB"));
        }
        let mut reader = crate::reader::Reader::new(&resource);
        let header = (reader.u16()? as i32 ^ ex.expr(&b, 0x263..0x278, &locals)?) as i16;
        let (key_length, shared_key) = if header == 0 {
            (
                (reader.u16()? as i32 ^ ex.expr(&b, 0x28a..0x2a1, &locals)?) as i16,
                vec![],
            )
        } else {
            if !(3..=1024).contains(&header) {
                return Err(stop("invalid shared key size"));
            }
            (0, reader.take(header as usize)?.to_vec())
        };
        if shared_key.is_empty() && key_length != -1 && !(3..=1024).contains(&key_length) {
            return Err(stop("invalid per-record key size"));
        }
        // Metadata-token/name hash, evaluated from metadata values, never reflection.
        let type_token = token_at(&hash, 0x9b)?;
        a.validate_token(type_token)?;
        let graph = body(a, token_at(&hash, 0xac)?)?;
        let mut gs = State::new();
        ex.run(&graph, 0..u32::MAX, &mut gs, &[], 0, false)?;
        if gs.stack.len() != 1 {
            return Err(stop("invalid metadata hash graph"));
        }
        let other = gs.integer()? as u32;
        let mut bytes: Vec<u8> = a
            .info
            .identity
            .name
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        bytes.extend(
            [
                type_token >> 24,
                other >> 16,
                type_token >> 8,
                other,
                type_token >> 16,
                other >> 8,
                type_token,
                other >> 24,
            ]
            .map(|n| n as u8),
        );
        let mut h = 0u64;
        for byte in bytes {
            h = h.wrapping_add(byte as u64);
            h = h.wrapping_add(h << 20);
            h ^= h >> 12;
        }
        h = h.wrapping_add(h << 6);
        h ^= h >> 22;
        h = h.wrapping_add(h << 30);
        let Operand::Integer(salt) = at(&hash, 0x174)?.operand else {
            return Err(stop("missing hash salt"));
        };
        h ^= salt as u64;
        let mix = ex.expr(&b, 0x306..0x311, &locals)?
            ^ h as i32
            ^ ex.expr(&b, 0x329..0x334, &locals)?
            ^ ex.expr(&b, 0x339..0x344, &locals)?
            ^ ex.expr(&b, 0x349..0x36b, &locals)?
            ^ ex.expr(&b, 0x370..0x386, &locals)?;
        // Read only scalar expressions from the initializer. Reflection-dependent
        // paths remain alternatives; no initializer instructions are executed.
        let mut ss = State::new();
        ex.run(&seed_body, 0..0xf, &mut ss, &[], 0, false)?;
        ss.locals.insert(0, Scalar::Int(0));
        let seed = ex.expr(&seed_body, 0x3c..0x65, &ss.locals)?;
        let frame = ex.expr(&seed_body, 0x7b..0x90, &ss.locals)?;
        let mut magics = BTreeSet::new();
        for flag in [0, frame] {
            for range in [0xbb..0xc7, 0xde..0xf3, 0x11b..0x125, 0x12a..0x141] {
                magics.insert(seed ^ flag ^ ex.expr(&seed_body, range, &ss.locals)? ^ mix);
            }
        }
        let p = Self {
            helper,
            resource,
            key_length,
            shared_key,
            magics: magics.into_iter().collect(),
            magic: None,
            offset_xor: ex.expr(&b, 0x433..0x43e, &locals)? ^ ex.expr(&b, 0x446..0x451, &locals)?,
            key_xor: ex.expr(&b, 0x48b..0x496, &locals)?,
            flags_xor: ex.expr(&b, 0x503..0x517, &locals)?,
            sentinel: ex.expr(&b, 0x51d..0x528, &locals)?,
            redirect: ex.expr(&b, 0x537..0x542, &locals)?,
            latin: ex.expr(&b, 0x571..0x57c, &locals)?,
            compressed: ex.expr(&b, 0x586..0x591, &locals)?,
            no_intern: ex.expr(&b, 0x59b..0x5a6, &locals)?,
            length_mask: ex.expr(&b, 0x5b0..0x5bb, &locals)?,
            cache: BTreeMap::new(),
            retained: 0,
            work: 0,
        };
        if (
            p.latin as u32,
            p.compressed as u32,
            p.no_intern as u32,
            p.length_mask,
        ) != (0x40000000, 0x80000000, 0x20000000, 0x0fffffff)
        {
            return Err(stop("unrecognized string record flags"));
        }
        Ok(Some(p))
    }
    pub fn decode(&mut self, arg: i32) -> Result<String> {
        if let Some(s) = self.cache.get(&arg) {
            return Ok(s.clone());
        }
        // The first record must identify exactly one seed. Ambiguity is rejected,
        // never resolved by guessing whether text looks readable.
        let candidates = if let Some(m) = self.magic {
            vec![m]
        } else {
            self.magics.clone()
        };
        let mut found = None;
        for magic in candidates {
            let result = self.record(arg, magic);
            if let Ok(text) = result {
                if found.is_some() {
                    return Err(stop("ambiguous resource seed"));
                }
                found = Some((magic, text));
            }
            if self.work > 32 * 1024 * 1024 {
                return Err(Error::limit("String decoding exceeded 32 MiB work budget"));
            }
        }
        let Some((magic, text)) = found else {
            return Err(stop("record did not validate; call retained"));
        };
        self.magic = Some(magic);
        // The cache only saves repeated work; a full cache never fails a record.
        if self.retained + text.len() <= MAX_TOTAL && self.cache.len() < 16_384 {
            self.retained += text.len();
            self.cache.insert(arg, text.clone());
        }
        Ok(text)
    }
    fn record(&mut self, mut arg: i32, magic: i32) -> Result<String> {
        let mut visited = BTreeSet::new();
        for _ in 0..16 {
            let offset = arg ^ self.offset_xor ^ magic;
            if offset < 4 || !visited.insert(offset) {
                return Err(stop("invalid offset or redirect cycle"));
            }
            let data = self
                .resource
                .get(offset as usize..)
                .ok_or_else(|| stop("record offset outside resource"))?;
            let mut r = crate::reader::Reader::new(data);
            let key = if self.shared_key.is_empty() {
                let n = if self.key_length == -1 {
                    (r.u16()? as i32 ^ self.key_xor ^ offset) as i16
                } else {
                    self.key_length
                };
                if !(3..=1024).contains(&n) {
                    return Err(stop("record key size out of bounds"));
                }
                let mut key = r.take(n as usize)?.to_vec();
                for (i, b) in key.iter_mut().enumerate() {
                    *b ^= (magic >> ((i & 3) * 8)) as u8;
                }
                key
            } else {
                self.shared_key.clone()
            };
            let flags = r.u32()? as i32 ^ offset ^ self.flags_xor ^ magic;
            if flags == self.sentinel {
                let b = r.take(4)?;
                let next = u32::from_le_bytes([b[2], b[0], b[3], b[1]]) as i32;
                arg = next ^ (self.redirect ^ magic).wrapping_neg();
                continue;
            }
            if flags & self.no_intern != 0 {
                return Err(stop("non-interned string retained"));
            }
            let n = (flags & self.length_mask) as usize;
            if n > MAX_TEXT {
                return Err(Error::limit("Decoded string record exceeds 1 MiB"));
            }
            let mut bytes = r.take(n)?.to_vec();
            self.work += bytes.len();
            if self.work > 32 * 1024 * 1024 {
                return Err(Error::limit("String work budget exceeded"));
            }
            decrypt(&key, &mut bytes)?;
            if flags & self.compressed != 0 {
                bytes = inflate(&bytes)?;
                self.work += bytes.len();
            }
            if flags & self.latin != 0 {
                return Ok(bytes.into_iter().map(char::from).collect());
            }
            if !bytes.len().is_multiple_of(2) {
                return Err(stop("odd UTF-16 byte count"));
            }
            return String::from_utf16(
                &bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| stop("unpaired UTF-16 surrogate"));
        }
        Err(stop("too many record redirects"))
    }
}
fn decrypt(key: &[u8], data: &mut [u8]) -> Result<()> {
    if key.len() < 3 {
        return Err(stop("short string key"));
    }
    let k = key[1];
    let mut previous = ((data.len() as u32 + 11) ^ (k as u32 + 7)) as u8;
    let mut state = (key[0] as u32 | ((key[2] as u32) << 8)) + ((previous as u32) << 3);
    let mut word = 0u16;
    for (i, byte) in data.iter_mut().enumerate() {
        if i & 1 == 0 {
            state = state.wrapping_mul(214013).wrapping_add(2531011);
            word = (state >> 16) as u16;
        }
        let low = word as u8;
        word >>= 8;
        let old = *byte;
        *byte = old ^ k ^ previous.wrapping_add(3) ^ low;
        previous = old;
    }
    Ok(())
}
fn inflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut r = crate::reader::Reader::new(bytes);
    let h = r.take(4)?;
    let n = u32::from_le_bytes([h[2], h[3], h[0], h[1]]) as usize;
    if n > MAX_TEXT {
        return Err(Error::limit("Inflated string exceeds 1 MiB"));
    }
    let mut out = Vec::with_capacity(n);
    let mut mask = 128u16;
    let mut bits = 0u8;
    while out.len() < n {
        mask <<= 1;
        if mask == 256 {
            mask = 1;
            bits = r.u8()?;
        }
        if bits as u16 & mask == 0 {
            out.push(r.u8()?);
        } else {
            let b = r.take(2)?;
            let count = (b[0] >> 2) as usize + 3;
            let distance = ((b[0] as usize) << 8 | b[1] as usize) & 1023;
            if distance == 0 || distance > out.len() {
                return Err(stop("invalid compressed back-reference"));
            }
            // The legacy loop stops at the declared output length.
            for _ in 0..count.min(n - out.len()) {
                out.push(out[out.len() - distance]);
            }
        }
    }
    if r.pos != bytes.len() {
        return Err(stop("trailing compressed string data"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        Profile {
            helper: 0x06000001,
            resource: vec![0; 4],
            key_length: 3,
            shared_key: vec![],
            magics: vec![0x12345678],
            magic: None,
            offset_xor: 19,
            key_xor: 37,
            flags_xor: 91,
            sentinel: -2,
            redirect: 127,
            latin: 0x40000000,
            compressed: i32::MIN,
            no_intern: 0x20000000,
            length_mask: 0x0fffffff,
            cache: BTreeMap::new(),
            retained: 0,
            work: 0,
        }
    }
    // Independent inverse encoder for owned synthetic records, not a protector.
    fn encrypt(key: [u8; 3], bytes: &[u8]) -> Vec<u8> {
        let mut state = (key[0] as u32 | ((key[2] as u32) << 8))
            + ((((bytes.len() + 11) ^ (key[1] as usize + 7)) as u8 as u32) << 3);
        let mut prev = ((bytes.len() + 11) ^ (key[1] as usize + 7)) as u8;
        let mut random = 0u16;
        let mut result = Vec::new();
        for (i, b) in bytes.iter().enumerate() {
            if i % 2 == 0 {
                state = state.wrapping_mul(214013).wrapping_add(2531011);
                random = (state >> 16) as u16;
            }
            let cipher = b ^ key[1] ^ prev.wrapping_add(3) ^ random as u8;
            random >>= 8;
            prev = cipher;
            result.push(cipher);
        }
        result
    }
    fn append(p: &mut Profile, bytes: &[u8], flags: i32) -> i32 {
        let offset = p.resource.len() as i32;
        let magic = p.magics[0];
        let key = [13, 37, 81];
        p.resource.extend(
            key.iter()
                .enumerate()
                .map(|(i, b)| b ^ (magic >> (i * 8)) as u8),
        );
        p.resource
            .extend(((bytes.len() as i32 | flags) ^ offset ^ p.flags_xor ^ magic).to_le_bytes());
        p.resource.extend(encrypt(key, bytes));
        offset ^ p.offset_xor ^ magic
    }
    #[test]
    fn resource_records_decode_unicode_latin_compression_and_deduplicate() {
        let mut p = profile();
        let u: Vec<_> = "Owned 😀 fixture"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let arg = append(&mut p, &u, 0);
        assert_eq!(p.decode(arg).unwrap(), "Owned 😀 fixture");
        let work = p.work;
        assert_eq!(p.decode(arg).unwrap(), "Owned 😀 fixture");
        assert_eq!(p.work, work);
        let arg = append(&mut p, &[b'A', 0xe9], 0x40000000);
        assert_eq!(p.decode(arg).unwrap(), "Aé");
        let packed = [0, 0, 9, 0, 8, b'a', b'b', b'c', 12, 3];
        let arg = append(&mut p, &packed, 0xc0000000u32 as i32);
        assert_eq!(p.decode(arg).unwrap(), "abcabcabc");
    }
    #[test]
    fn malformed_records_never_guess_or_allocate_without_bounds() {
        let mut p = profile();
        let arg = append(&mut p, b"test", 0x40000000);
        let original = p.resource.clone();
        for length in 0..original.len() {
            let mut truncated = profile();
            truncated.resource = original[..length].to_vec();
            assert!(truncated.decode(arg).is_err(), "{length}");
        }
        let arg = append(&mut p, b"new string", 0x60000000);
        assert!(p.decode(arg).is_err());
        let arg = append(&mut p, &[0, 0xd8], 0);
        assert!(p.decode(arg).is_err());
        assert!(inflate(&[0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(inflate(&[0, 0, 3, 0, 1, 0, 0]).is_err()); // zero distance
        assert!(inflate(&[0, 0, 3, 0, 1, 0, 1]).is_err()); // before output start
        assert!(decrypt(&[], &mut [0]).is_err());
        p.work = 32 * 1024 * 1024;
        let arg = append(&mut p, b"budget", 0x40000000);
        assert!(p.decode(arg).is_err());
    }
    #[test]
    fn redirect_cycles_and_ambiguous_seeds_are_rejected() {
        let mut p = profile();
        let magic = p.magics[0];
        let offset = 4i32;
        let arg = offset ^ p.offset_xor ^ magic;
        p.resource.extend([1, 2, 3]);
        p.resource
            .extend((p.sentinel ^ offset ^ p.flags_xor ^ magic).to_le_bytes());
        let v = (arg ^ (p.redirect ^ magic).wrapping_neg()).to_le_bytes();
        p.resource.extend([v[1], v[3], v[0], v[2]]);
        assert!(p.decode(arg).is_err());
        let mut p = profile();
        let arg = append(&mut p, b"x", 0x40000000);
        p.magics.push(p.magics[0]);
        assert!(p.decode(arg).unwrap_err().detail.contains("ambiguous"));
    }
    fn synthetic(code: &[u8]) -> MethodBody {
        MethodBody {
            max_stack: 16,
            init_locals: false,
            local_signature: 0,
            code_size: code.len() as u32,
            file_offset: 0,
            instructions: crate::cil::decode(code).unwrap(),
            exceptions: vec![],
        }
    }
    #[test]
    fn scalar_extraction_is_closed_bounded_and_wraps_int32() {
        let a = Assembly::load(
            include_bytes!("../../../../samples/fixtures/ILens.Deobfuscation.dll").to_vec(),
        )
        .unwrap();
        let mut ex = Expressions { a: &a, steps: 0 };
        let mut st = State::new();
        let b = synthetic(&[0x20, 0xff, 0xff, 0xff, 0x7f, 0x17, 0x58, 0x2a]);
        ex.run(&b, 0..u32::MAX, &mut st, &[], 0, false).unwrap();
        assert_eq!(st.integer().unwrap(), i32::MIN);
        let b = synthetic(&[0x7e, 1, 0, 0, 4, 0x2a]);
        assert!(
            ex.run(&b, 0..u32::MAX, &mut State::new(), &[], 0, false)
                .is_err()
        ); // fields unavailable
        let b = synthetic(&[0x20, 1, 0, 0, 0, 0x2a]);
        ex.steps = 20_000;
        assert!(
            ex.run(&b, 0..u32::MAX, &mut State::new(), &[], 0, false)
                .is_err()
        );
        assert!(Profile::detect(&a).unwrap().is_none());
    }
}
