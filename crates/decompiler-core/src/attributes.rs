//! Custom attribute rendering for display views (ECMA-335 II.23.3). Blobs are decoded
//! as data only; an undecodable blob still shows the attribute name.
use crate::{
    assembly::{Assembly, coded, strip_arity},
    error::{Error, Result},
    reader::Reader,
    signature::Type,
};
// Compiler bookkeeping that dnSpy/ILSpy express as syntax or leave out of C# views.
const HIDDEN: &[&str] = &[
    "NullableAttribute",
    "NullableContextAttribute",
    "NullablePublicOnlyAttribute",
    "IsReadOnlyAttribute",
    "IsByRefLikeAttribute",
    "CompilerFeatureRequiredAttribute",
    "RefSafetyRulesAttribute",
    "ScopedRefAttribute",
    // Written as the async/iterator body itself.
    "AsyncStateMachineAttribute",
    "IteratorStateMachineAttribute",
    "AsyncIteratorStateMachineAttribute",
    // Written as C# syntax: `this`, `params`, indexers, tuple element names.
    "ExtensionAttribute",
    "ParamArrayAttribute",
    "DefaultMemberAttribute",
    "TupleElementNamesAttribute",
];
fn short(name: &str) -> &str {
    name.rsplit(['.', '+']).next().unwrap_or(name)
}
fn ser_string(r: &mut Reader<'_>) -> Result<Option<String>> {
    if r.data.get(r.pos) == Some(&0xff) {
        r.u8()?;
        return Ok(None);
    }
    let length = r.compressed()? as usize;
    if length > 64 * 1024 {
        return Err(Error::limit("Attribute string exceeds 64 KiB"));
    }
    Ok(Some(
        std::str::from_utf8(r.take(length)?)
            .map_err(|_| Error::metadata("Invalid UTF-8 in attribute"))?
            .to_owned(),
    ))
}
fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}
// A serialized type name such as "System.String, mscorlib, Version=…" as typeof(String).
fn type_name(text: &str) -> String {
    let name = text.split(',').next().unwrap_or(text).trim();
    let name = short(name).split('`').next().unwrap_or(name);
    match name {
        "String" => "string".into(),
        "Int32" => "int".into(),
        "Boolean" => "bool".into(),
        "Object" => "object".into(),
        other => other.to_owned(),
    }
}
/// Element encodings used by both fixed and named arguments.
enum Element {
    Primitive(u8),
    Type,
    Boxed,
    /// Name, storage width and members (when known).
    Enum(String, u8, Option<Vec<(String, i128)>>),
    Array(Box<Element>),
}
// Enum storage width: local enums are read from value__, other enums default to int32.
fn enum_size(a: &Assembly, token: u32) -> u8 {
    if token >> 24 == 2
        && let Some(field) = a
            .nodes
            .iter()
            .find(|n| n.parent == token && n.name == "value__")
        && let Ok((_, _, Type::Primitive(name))) = a.field_ref(field.token)
    {
        return match name.as_str() {
            "sbyte" => 0x04,
            "byte" => 0x05,
            "short" => 0x06,
            "ushort" => 0x07,
            "uint" => 0x09,
            "long" => 0x0a,
            "ulong" => 0x0b,
            _ => 0x08,
        };
    }
    0x08
}
fn from_type(a: &Assembly, ty: &Type) -> Result<Element> {
    Ok(match ty {
        Type::Primitive(name) => Element::Primitive(match name.as_str() {
            "bool" => 0x02,
            "char" => 0x03,
            "sbyte" => 0x04,
            "byte" => 0x05,
            "short" => 0x06,
            "ushort" => 0x07,
            "int" => 0x08,
            "uint" => 0x09,
            "long" => 0x0a,
            "ulong" => 0x0b,
            "float" => 0x0c,
            "double" => 0x0d,
            "string" => 0x0e,
            "object" => return Ok(Element::Boxed),
            _ => return Err(Error::limitation("Unsupported attribute parameter type")),
        }),
        Type::Named(t) if matches!(a.resolve(*t).as_str(), "System.Type" | "Type") => Element::Type,
        Type::ValueType(t) => Element::Enum(
            strip_arity(&a.resolve(*t)),
            enum_size(a, *t),
            crate::bcl::members_of(a, *t),
        ),
        Type::Array(inner, 1) => Element::Array(Box::new(from_type(a, inner)?)),
        _ => return Err(Error::limitation("Unsupported attribute parameter type")),
    })
}
fn from_tag(r: &mut Reader<'_>, depth: usize) -> Result<Element> {
    if depth > 8 {
        return Err(Error::limit("Attribute type nesting exceeds 8"));
    }
    Ok(match r.u8()? {
        tag @ 0x02..=0x0e => Element::Primitive(tag),
        0x50 => Element::Type,
        0x51 => Element::Boxed,
        0x55 => {
            let serialized = ser_string(r)?.unwrap_or_default();
            Element::Enum(
                type_name(&serialized),
                0x08,
                crate::bcl::members_by_name(&serialized).map(<[_]>::to_vec),
            )
        }
        0x1d => Element::Array(Box::new(from_tag(r, depth + 1)?)),
        _ => return Err(Error::metadata("Unknown attribute element type")),
    })
}
fn value(r: &mut Reader<'_>, element: &Element, depth: usize) -> Result<String> {
    if depth > 8 {
        return Err(Error::limit("Attribute value nesting exceeds 8"));
    }
    Ok(match element {
        Element::Primitive(tag) => match tag {
            0x02 => if r.u8()? != 0 { "true" } else { "false" }.to_owned(),
            0x03 => {
                let c = char::from_u32(r.u16()? as u32).unwrap_or('?');
                format!("'{}'", c.escape_default())
            }
            0x04 => (r.u8()? as i8).to_string(),
            0x05 => r.u8()?.to_string(),
            0x06 => (r.u16()? as i16).to_string(),
            0x07 => r.u16()?.to_string(),
            0x08 => (r.u32()? as i32).to_string(),
            0x09 => format!("{}U", r.u32()?),
            0x0a => format!("{}L", r.u64()? as i64),
            0x0b => format!("{}UL", r.u64()?),
            0x0c => format!("{}f", f32::from_bits(r.u32()?)),
            0x0d => f64::from_bits(r.u64()?).to_string(),
            0x0e => ser_string(r)?.map_or("null".to_owned(), |s| quote(&s)),
            _ => return Err(Error::metadata("Unknown attribute primitive")),
        },
        Element::Type => {
            ser_string(r)?.map_or("null".to_owned(), |s| format!("typeof({})", type_name(&s)))
        }
        Element::Boxed => {
            let inner = from_tag(r, depth + 1)?;
            value(r, &inner, depth + 1)?
        }
        Element::Enum(name, size, members) => {
            let raw = value(r, &Element::Primitive(*size), depth + 1)?;
            let raw = raw.trim_end_matches(['U', 'L']);
            match (members, raw.parse::<i128>()) {
                (Some(members), Ok(number)) => crate::bcl::enum_text(short(name), members, number),
                _ => format!("({}){raw}", short(name)),
            }
        }
        Element::Array(inner) => {
            let count = r.u32()?;
            if count == u32::MAX {
                return Ok("null".to_owned());
            }
            if count > 4096 {
                return Err(Error::limit("Attribute array exceeds 4096 elements"));
            }
            let items = (0..count)
                .map(|_| value(r, inner, depth + 1))
                .collect::<Result<Vec<_>>>()?;
            format!("new {}[] {{ {} }}", element_name(inner), items.join(", "))
        }
    })
}
fn element_name(element: &Element) -> String {
    match element {
        Element::Primitive(tag) => match tag {
            0x02 => "bool",
            0x03 => "char",
            0x04 => "sbyte",
            0x05 => "byte",
            0x06 => "short",
            0x07 => "ushort",
            0x08 => "int",
            0x09 => "uint",
            0x0a => "long",
            0x0b => "ulong",
            0x0c => "float",
            0x0d => "double",
            _ => "string",
        }
        .to_owned(),
        Element::Type => "Type".to_owned(),
        Element::Boxed => "object".to_owned(),
        Element::Enum(name, ..) => short(name).to_owned(),
        Element::Array(inner) => format!("{}[]", element_name(inner)),
    }
}
fn arguments(a: &Assembly, constructor: u32, blob: &[u8]) -> Result<String> {
    let signature = a.method_ref(constructor)?.signature;
    let mut r = Reader::new(blob);
    if r.u16()? != 1 {
        return Err(Error::metadata("Invalid custom attribute prolog"));
    }
    let mut args = vec![];
    for parameter in &signature.parameters {
        args.push(value(&mut r, &from_type(a, parameter)?, 0)?);
    }
    let named = r.u16()?;
    if named > 256 {
        return Err(Error::limit("Too many named attribute arguments"));
    }
    for _ in 0..named {
        if !matches!(r.u8()?, 0x53 | 0x54) {
            return Err(Error::metadata("Invalid named attribute argument"));
        }
        let element = from_tag(&mut r, 0)?;
        let name = ser_string(&mut r)?.unwrap_or_default();
        args.push(format!("{name} = {}", value(&mut r, &element, 0)?));
    }
    if r.pos != blob.len() {
        return Err(Error::metadata("Trailing custom attribute bytes"));
    }
    Ok(args.join(", "))
}
/// `[Name(args)]` lines for a declaration, in metadata order.
pub fn render(a: &Assembly, parent: u32) -> Vec<String> {
    let mut lines = vec![];
    let flags = match parent >> 24 {
        2 => a
            .metadata()
            .type_defs
            .get((parent & 0xffffff) as usize - 1)
            .map(|t| t.flags),
        _ => None,
    };
    // TypeAttributes.Serializable is stored as a flag, not as an attribute row.
    if flags.is_some_and(|f| f & 0x2000 != 0) {
        lines.push("[Serializable]".to_owned());
    }
    if parent >> 24 == 4
        && a.metadata()
            .fields
            .get((parent & 0xffffff) as usize - 1)
            .is_some_and(|f| f.flags & 0x80 != 0)
    {
        lines.push("[NonSerialized]".to_owned());
    }
    for index in a.attributes.get(&parent).into_iter().flatten().take(256) {
        let attribute = &a.metadata().custom_attributes[*index];
        let constructor = coded(attribute.attr_type);
        let Ok(method) = a.method_ref(constructor) else {
            continue;
        };
        // Check hidden attributes by full name, so they add no `using` to the view.
        if HIDDEN.contains(&short(&strip_arity(&a.resolve_full(method.owner)))) {
            continue;
        }
        let owner = strip_arity(&a.resolve(method.owner));
        if HIDDEN.contains(&short(&owner)) {
            continue;
        }
        let name = owner
            .strip_suffix("Attribute")
            .filter(|n| !n.is_empty() && !n.ends_with('.'))
            .unwrap_or(&owner);
        let rendered = a
            .blob(attribute.value)
            .and_then(|blob| arguments(a, constructor, blob));
        lines.push(match rendered {
            Ok(args) if args.is_empty() => format!("[{name}]"),
            Ok(args) => format!("[{name}({args})]"),
            Err(_) => format!("[{name}] // arguments could not be decoded"),
        });
    }
    lines
}
/// True when the declaration carries [CompilerGenerated].
/// True when `parent` carries an attribute whose type has this full name.
pub fn has(a: &Assembly, parent: u32, full_name: &str) -> bool {
    a.attributes
        .get(&parent)
        .into_iter()
        .flatten()
        .any(|index| {
            let constructor = coded(a.metadata().custom_attributes[*index].attr_type);
            a.method_ref(constructor)
                .is_ok_and(|m| a.resolve_full(m.owner) == full_name)
        })
}
/// The member named by a type's [DefaultMember("…")], which C# exposes as its indexer.
pub fn default_member(a: &Assembly, ty: u32) -> Option<String> {
    a.attributes
        .get(&ty)
        .into_iter()
        .flatten()
        .find_map(|index| {
            let attribute = &a.metadata().custom_attributes[*index];
            let constructor = a.method_ref(coded(attribute.attr_type)).ok()?;
            if a.resolve_full(constructor.owner) != "System.Reflection.DefaultMemberAttribute" {
                return None;
            }
            let mut r = Reader::new(a.blob(attribute.value).ok()?);
            (r.u16().ok()? == 1).then_some(())?;
            ser_string(&mut r).ok()?
        })
}
pub fn compiler_generated(a: &Assembly, parent: u32) -> bool {
    a.attributes
        .get(&parent)
        .into_iter()
        .flatten()
        .any(|index| {
            let constructor = coded(a.metadata().custom_attributes[*index].attr_type);
            a.method_ref(constructor).is_ok_and(|m| {
                a.resolve_full(m.owner)
                    .ends_with("CompilerGeneratedAttribute")
            })
        })
}
