//! Bounded metadata-only framework and obfuscation inspection. No method bodies
//! are evaluated or decoded here. Markers are evidence, never proof of provenance.
use crate::{
    assembly::{Assembly, Node, coded},
    error::{Error, Result},
    reader::Reader,
    signature::Type,
};
use serde::Serialize;
use std::collections::HashMap;

const MAX_ATTRIBUTES: usize = 4096;
const MAX_EVIDENCE: usize = 16;
const TARGET_FRAMEWORK: &str = "System.Runtime.Versioning.TargetFrameworkAttribute";

#[derive(Debug, Clone, Serialize)]
pub struct FrameworkInfo {
    pub display_name: String,
    pub moniker: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub status: &'static str,
}
impl Default for FrameworkInfo {
    fn default() -> Self {
        Self {
            display_name: "Not declared".into(),
            moniker: None,
            version: None,
            source: None,
            status: "not_declared",
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub kind: &'static str,
    pub tool: Option<&'static str>,
    pub confidence: &'static str,
    pub detail: String,
    pub token: Option<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ObfuscationInfo {
    pub status: &'static str,
    pub evidence: Vec<Evidence>,
    pub scan_complete: bool,
}
impl Default for ObfuscationInfo {
    fn default() -> Self {
        Self {
            status: "no_markers",
            evidence: vec![],
            scan_complete: true,
        }
    }
}

// ECMA-335 II.23.3: prolog, one fixed SerString argument, named argument count.
// Accept only string-valued named properties/fields for these known attributes.
// Parsing the entire bounded blob prevents truncated/spoofed prefixes from being
// reported as valid framework declarations.
fn ser_string(reader: &mut Reader<'_>) -> Result<Option<String>> {
    if reader.data.get(reader.pos) == Some(&0xff) {
        reader.u8()?;
        return Ok(None);
    }
    let length = reader.compressed()? as usize;
    if length > 4096 {
        return Err(Error::limit("Attribute string exceeds 4096 bytes"));
    }
    let value = std::str::from_utf8(reader.take(length)?)
        .map_err(|_| Error::metadata("Invalid UTF-8 in attribute string"))?;
    if value.chars().any(char::is_control) {
        return Err(Error::metadata("Control character in attribute string"));
    }
    Ok(Some(value.to_owned()))
}
fn string_attribute(blob: &[u8]) -> Result<String> {
    if blob.len() > 16 * 1024 {
        return Err(Error::limit("Inspection attribute exceeds 16 KiB"));
    }
    let mut reader = Reader::new(blob);
    if reader.u16()? != 1 {
        return Err(Error::metadata("Invalid custom attribute prolog"));
    }
    let value = ser_string(&mut reader)?.ok_or_else(|| Error::metadata("Null attribute string"))?;
    let count = reader.u16()?;
    if count > 16 {
        return Err(Error::limit(
            "Too many named inspection attribute arguments",
        ));
    }
    for _ in 0..count {
        if !matches!(reader.u8()?, 0x53 | 0x54) || reader.u8()? != 0x0e {
            return Err(Error::metadata(
                "Unsupported named inspection attribute argument",
            ));
        }
        ser_string(&mut reader)?.ok_or_else(|| Error::metadata("Missing named argument name"))?;
        ser_string(&mut reader)?;
    }
    if reader.pos != blob.len() {
        return Err(Error::metadata("Trailing custom attribute bytes"));
    }
    Ok(value)
}
fn framework(moniker: &str) -> Result<FrameworkInfo> {
    let mut parts = moniker.split(',').map(str::trim);
    let family = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::metadata("Empty target framework"))?;
    let mut version = None;
    let mut profile = None;
    for part in parts {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| Error::metadata("Invalid framework moniker"))?;
        match key.trim().to_ascii_lowercase().as_str() {
            "version" if version.is_none() => {
                version = Some(value.trim().trim_start_matches(['v', 'V']))
            }
            "profile" if profile.is_none() => profile = Some(value.trim()),
            _ => {
                return Err(Error::metadata(
                    "Duplicate or unsupported framework moniker field",
                ));
            }
        }
    }
    let version = version.ok_or_else(|| Error::metadata("Missing target framework version"))?;
    let numbers: Vec<_> = version.split('.').collect();
    if !(2..=4).contains(&numbers.len())
        || numbers.iter().any(|n| {
            n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || n.parse::<u16>().is_err()
        })
    {
        return Err(Error::metadata("Invalid target framework version"));
    }
    let family_name = match family {
        ".NETCoreApp" if numbers[0].parse::<u16>().unwrap_or(0) >= 5 => ".NET",
        ".NETCoreApp" => ".NET Core",
        ".NETFramework" => ".NET Framework",
        ".NETStandard" => ".NET Standard",
        other => other,
    };
    let suffix = profile
        .filter(|s| !s.is_empty())
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    Ok(FrameworkInfo {
        display_name: format!("{family_name} {version}{suffix}"),
        moniker: Some(moniker.into()),
        version: Some(version.into()),
        source: Some("TargetFrameworkAttribute".into()),
        status: "declared",
    })
}
fn marker(name: &str, payload: Option<&str>) -> Option<&'static str> {
    match name {
        "ConfusedByAttribute" => Some(if payload.is_some_and(|p| p.starts_with("ConfuserEx")) {
            "ConfuserEx"
        } else {
            "Confuser family"
        }),
        "DotfuscatorAttribute" => Some("Dotfuscator"),
        "SmartAssembly.Attributes.PoweredByAttribute" => Some("SmartAssembly"),
        "BabelAttribute" | "BabelObfuscatorAttribute" => Some("Babel Obfuscator"),
        _ => None,
    }
}
fn hidden_character(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}
fn name_evidence(nodes: &[Node]) -> Vec<Evidence> {
    let mut eligible = 0;
    let mut short = 0;
    let mut hidden = 0;
    let mut first_hidden = None;
    for n in nodes {
        if !matches!(n.token >> 24, 2 | 4 | 6 | 20 | 23) {
            continue;
        }
        if n.name.chars().any(hidden_character) {
            hidden += 1;
            first_hidden.get_or_insert(n.token);
        }
        // Common compiler-generated names, accessors, and constructors do not
        // count towards the short-name heuristic. Non-Latin identifiers are valid.
        if n.name.starts_with(['<', '$', '.'])
            || n.name.starts_with("get_")
            || n.name.starts_with("set_")
            || n.name.starts_with("add_")
            || n.name.starts_with("remove_")
        {
            continue;
        }
        eligible += 1;
        if !n.name.is_empty()
            && n.name.len() <= 2
            && n.name.bytes().all(|b| b.is_ascii_alphabetic())
        {
            short += 1
        }
    }
    let mut evidence = vec![];
    if hidden > 0 {
        evidence.push(Evidence {kind: "hidden_identifier_characters", tool: None, confidence: "heuristic", detail: format!("{hidden} declaration names contain control, invisible, or bidirectional formatting characters. This can indicate renaming; it does not identify a particular tool."), token: first_hidden});
    }
    if eligible >= 50 && short * 100 >= eligible * 80 {
        evidence.push(Evidence {kind: "short_identifier_ratio", tool: None, confidence: "heuristic", detail: format!("{short} of {eligible} eligible declarations have one- or two-letter ASCII names. This can indicate renaming, but compact naming conventions can produce the same result."), token: None});
    }
    evidence
}

pub fn inspect(a: &Assembly) -> (FrameworkInfo, ObfuscationInfo, Vec<String>) {
    let mut target = FrameworkInfo::default();
    let mut obfuscation = ObfuscationInfo::default();
    let mut diagnostics = vec![];
    let mut constructors = HashMap::new();
    let mut framework_count = 0;
    let mut inspected = 0;
    for (index, attribute) in a.metadata().custom_attributes.iter().enumerate() {
        // Applied assembly/module attributes only: a string or an unused type
        // named after a protector is not sufficient evidence.
        if !matches!(coded(attribute.parent), 0x20000001 | 0x00000001) {
            continue;
        }
        if inspected == MAX_ATTRIBUTES {
            obfuscation.scan_complete = false;
            diagnostics.push(
                "Framework and obfuscation inspection stopped at 4096 assembly/module attributes."
                    .into(),
            );
            break;
        }
        inspected += 1;
        let constructor = coded(attribute.attr_type);
        let descriptor = constructors.entry(constructor).or_insert_with(|| {
            a.method_ref(constructor).ok().filter(|m| m.name == ".ctor" && m.signature.has_this).map(|m| {
                let string_parameter = m.signature.parameters.len() == 1 && matches!(&m.signature.parameters[0], Type::Primitive(name) if name == "string");
                (a.resolve(m.owner), string_parameter)
            })
        });
        let Some((name, string_parameter)) = descriptor else {
            obfuscation.scan_complete = false;
            continue;
        };
        if name == TARGET_FRAMEWORK && coded(attribute.parent) == 0x20000001 {
            framework_count += 1;
            match a.blob(attribute.value).and_then(|blob| {
                if !*string_parameter {
                    return Err(Error::metadata("Expected TargetFrameworkAttribute(string)"));
                }
                framework(&string_attribute(blob)?)
            }) {
                Ok(info) => target = info,
                Err(_) => {
                    target.status = "invalid";
                    target.display_name = "Unavailable".into();
                    if !diagnostics
                        .iter()
                        .any(|s: &String| s.starts_with("The target framework"))
                    {
                        diagnostics.push("The target framework attribute is invalid or exceeds inspection limits; the exact .NET target is unavailable.".into());
                    }
                }
            }
        }
        if marker(name, None).is_none() {
            continue;
        }
        let payload = if *string_parameter {
            a.blob(attribute.value)
                .ok()
                .and_then(|b| string_attribute(b).ok())
        } else {
            None
        };
        let tool = marker(name, payload.as_deref());
        if obfuscation.evidence.iter().any(|e| e.tool == tool) {
            continue;
        }
        if obfuscation.evidence.len() >= MAX_EVIDENCE {
            obfuscation.scan_complete = false;
            continue;
        }
        let description = payload
            .as_ref()
            .map(|s| format!("; value: {}", s.chars().take(160).collect::<String>()))
            .unwrap_or_default();
        obfuscation.evidence.push(Evidence {kind: "protector_attribute", tool, confidence: "marker", detail: format!("Applied {name} attribute{description}. This identifies a marker, not verified tool provenance."), token: Some(0x0c000001 + index as u32)});
    }
    if framework_count > 1 {
        target = FrameworkInfo {
            status: "ambiguous",
            display_name: "Conflicting declarations".into(),
            ..FrameworkInfo::default()
        };
        diagnostics.push("Multiple TargetFrameworkAttribute declarations were found; the exact .NET target is ambiguous.".into());
    } else if !obfuscation.scan_complete && framework_count == 0 {
        target.status = "incomplete";
        target.display_name = "Inspection incomplete".into();
    }
    obfuscation.evidence.extend(name_evidence(&a.nodes));
    obfuscation.status = if obfuscation
        .evidence
        .iter()
        .any(|e| e.confidence == "marker")
    {
        "markers_found"
    } else if !obfuscation.evidence.is_empty() {
        "possible"
    } else if !obfuscation.scan_complete {
        "inconclusive"
    } else {
        "no_markers"
    };
    (target, obfuscation, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framework_versions_are_not_clr_metadata_versions() {
        for (moniker, label) in [
            (".NETFramework,Version=v4.8", ".NET Framework 4.8"),
            (
                ".NETFramework,Version=v4.0,Profile=Client",
                ".NET Framework 4.0 (Client)",
            ),
            (".NETCoreApp,Version=v3.1", ".NET Core 3.1"),
            (".NETCoreApp,Version=v10.0", ".NET 10.0"),
            (".NETStandard,Version=v2.0", ".NET Standard 2.0"),
        ] {
            assert_eq!(framework(moniker).unwrap().display_name, label)
        }
        for invalid in [
            "v4.0.30319",
            ".NETFramework",
            ".NETCoreApp,Version=v9",
            ".NETCoreApp,Version=v10.0,Version=v9.0",
            ".NETCoreApp,Version=v10.foo",
        ] {
            assert!(framework(invalid).is_err())
        }
    }
    #[test]
    fn bounded_attribute_strings_reject_all_truncations() {
        let text = ".NETCoreApp,Version=v10.0";
        let mut blob = vec![1, 0, text.len() as u8];
        blob.extend(text.as_bytes());
        blob.extend([0, 0]);
        assert_eq!(string_attribute(&blob).unwrap(), text);
        for i in 0..blob.len() {
            assert!(string_attribute(&blob[..i]).is_err(), "truncation {i}")
        }
        for invalid in [
            vec![1, 0, 0xff, 0, 0],
            vec![1, 0, 1, 0xff, 0, 0],
            vec![1, 0, 0xe0],
            vec![0; 20000],
        ] {
            assert!(string_attribute(&invalid).is_err())
        }
    }
    #[test]
    fn obfuscation_configuration_and_normal_names_are_not_markers() {
        assert!(marker("System.Reflection.ObfuscationAttribute", None).is_none());
        assert!(marker("Example.DotfuscatorAttribute", None).is_none());
        assert_eq!(
            marker("ConfusedByAttribute", Some("ConfuserEx v1.0.0")),
            Some("ConfuserEx")
        );
        let node = |name: &str| Node {
            name: name.into(),
            token: 0x06000001,
            parent: 0,
            kind: "method".into(),
            namespace: "".into(),
            full_name: name.into(),
            flags: 0,
        };
        assert!(
            name_evidence(&[
                node("MoveNext"),
                node("<Run>d__1"),
                node("Hesapla"),
                node("计算")
            ])
            .is_empty()
        );
        assert_eq!(
            name_evidence(&[node("a\u{202e}b")])[0].confidence,
            "heuristic"
        );
        assert!(name_evidence(&vec![node("a"); 49]).is_empty());
        assert_eq!(
            name_evidence(&vec![node("a"); 50])[0].kind,
            "short_identifier_ratio"
        );
    }
}
