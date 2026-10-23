//! Static packer/protector detection in the spirit of Detect It Easy. Every finding
//! carries its evidence; signatures identify markers, never verified provenance.
//! Nothing is executed or unpacked. Native files are scanned from their raw bytes.
use crate::{
    assembly::Assembly,
    error::Result,
    headers::entropy,
    reader::{slice, u16_at, u32_at},
};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub name: &'static str,
    /// packer, protector, obfuscator, bundler or virtualizer.
    pub kind: &'static str,
    /// "signature" for a specific marker, "heuristic" for generic indicators.
    pub confidence: &'static str,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub kind: &'static str,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct DetectionReport {
    pub managed: bool,
    /// packed, protected, obfuscated, suspicious or no_findings.
    pub verdict: &'static str,
    pub detections: Vec<Detection>,
    pub signals: Vec<Signal>,
}
struct RawSection {
    name: String,
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: usize,
    raw_size: usize,
    characteristics: u32,
}
struct Raw {
    sections: Vec<RawSection>,
    entry: u32,
    end_of_sections: usize,
}
// Independent bounded section parsing so native and malformed images can be scanned.
fn raw(bytes: &[u8]) -> Result<Raw> {
    let pe = u32_at(bytes, 0x3c)? as usize;
    if slice(bytes, pe, 4)? != b"PE\0\0" {
        return Err(crate::error::Error::new(
            crate::error::ErrorCode::InvalidPe,
            "Missing PE signature",
        ));
    }
    let count = (u16_at(bytes, pe + 6)? as usize).min(96);
    let optional = pe + 24;
    let optional_size = u16_at(bytes, pe + 20)? as usize;
    let entry = u32_at(bytes, optional + 16)?;
    let headers = optional + optional_size;
    let mut sections = vec![];
    let mut end = 0;
    for i in 0..count {
        let h = headers + i * 40;
        let Ok(header) = slice(bytes, h, 40) else {
            break;
        };
        let raw_offset = (u32_at(header, 20)? as usize).min(bytes.len());
        let raw_size = (u32_at(header, 16)? as usize).min(bytes.len() - raw_offset);
        if raw_size > 0 {
            end = end.max(raw_offset + raw_size);
        }
        sections.push(RawSection {
            name: String::from_utf8_lossy(&header[..8])
                .trim_end_matches('\0')
                .to_owned(),
            virtual_size: u32_at(header, 8)?,
            virtual_address: u32_at(header, 12)?,
            raw_offset,
            raw_size,
            characteristics: u32_at(header, 36)?,
        });
    }
    Ok(Raw {
        sections,
        entry,
        end_of_sections: end,
    })
}
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
// Section-name signatures of well-known native packers and protectors.
const SECTION_SIGNATURES: &[(&str, &str, &[&str])] = &[
    ("UPX", "packer", &["UPX0", "UPX1", "UPX2"]),
    ("MPRESS", "packer", &[".MPRESS1", ".MPRESS2"]),
    ("ASPack", "packer", &[".aspack", ".adata"]),
    ("PECompact", "packer", &["PEC2", "PEC2TO", "pec1", "pec2"]),
    ("Petite", "packer", &[".petite"]),
    ("NsPack", "packer", &[".nsp0", ".nsp1", "nsp0", "nsp1"]),
    ("FSG", "packer", &["FSG!"]),
    (
        "Themida / WinLicense",
        "protector",
        &[".themida", ".winlice", ".taggant"],
    ),
    ("VMProtect", "virtualizer", &[".vmp0", ".vmp1", ".vmp2"]),
    ("Enigma Protector", "protector", &[".enigma1", ".enigma2"]),
    ("Obsidium", "protector", &[".obsidium"]),
];
// Types or namespaces that protectors inject into the protected module itself. A type
// name alone can be copied, so these are heuristics; applied attributes are signatures.
const TYPE_SIGNATURES: &[(&str, &str, &str)] = &[
    ("Agile.NET (CliSecure)", "protector", "AgileDotNetRT"),
    ("Costura.Fody", "bundler", "Costura.AssemblyLoader"),
    ("Goliath.NET", "obfuscator", "ObfuscatedByGoliath"),
    ("Crypto Obfuscator", "obfuscator", "CryptoObfuscator."),
    ("KoiVM", "virtualizer", "KoiVM."),
    ("Fody", "bundler", "ProcessedByFody"),
];
// SHA-256 marker the .NET host writer stores in single-file bundle apphosts.
const BUNDLE_SIGNATURE: [u8; 32] = [
    0x8b, 0x12, 0x02, 0xb9, 0x6a, 0x61, 0x20, 0x38, 0x72, 0x7b, 0x93, 0x02, 0x14, 0xd7, 0xa0, 0x32,
    0x13, 0xf5, 0xb9, 0xe6, 0xef, 0xae, 0x33, 0x18, 0xee, 0x3b, 0x2d, 0xce, 0x24, 0xb3, 0x6a, 0xae,
];
const HIGH_ENTROPY: f64 = 7.2;
fn add(
    detections: &mut Vec<Detection>,
    name: &'static str,
    kind: &'static str,
    confidence: &'static str,
    evidence: String,
) {
    if let Some(existing) = detections.iter_mut().find(|d| d.name == name) {
        if existing.evidence.len() < 8 && !existing.evidence.contains(&evidence) {
            existing.evidence.push(evidence);
        }
        if confidence == "signature" {
            existing.confidence = "signature";
        }
        return;
    }
    detections.push(Detection {
        name,
        kind,
        confidence,
        evidence: vec![evidence],
    });
}
pub fn scan(bytes: &[u8], assembly: Option<&Assembly>) -> Result<DetectionReport> {
    let image = raw(bytes)?;
    let mut detections = vec![];
    let mut signals = vec![];
    for s in &image.sections {
        for (tool, kind, names) in SECTION_SIGNATURES {
            if names.iter().any(|n| s.name == *n) {
                add(
                    &mut detections,
                    tool,
                    kind,
                    "signature",
                    format!("Section named {}", s.name),
                );
            }
        }
    }
    let prefix = &bytes[..bytes.len().min(4096)];
    if contains(prefix, b"UPX!") {
        add(
            &mut detections,
            "UPX",
            "packer",
            "signature",
            "UPX! header marker".into(),
        );
    }
    if bytes.len() <= 64 * 1024 * 1024 && contains(bytes, &BUNDLE_SIGNATURE) {
        add(&mut detections, ".NET single-file bundle", "bundler", "signature", "Host bundle signature found; the managed assemblies are embedded in the apphost and must be extracted to decompile them".into());
    }
    let mut high = 0;
    for s in &image.sections {
        let data = &bytes[s.raw_offset..s.raw_offset + s.raw_size];
        let e = entropy(data);
        let executable = s.characteristics & 0x2000_0000 != 0;
        let writable = s.characteristics & 0x8000_0000 != 0;
        if s.raw_size >= 4096 && e >= HIGH_ENTROPY {
            high += 1;
            signals.push(Signal {kind: "high_entropy_section", detail: format!("Section {} has entropy {e:.2} over {} bytes, typical of compressed or encrypted data", display(&s.name), s.raw_size)});
        }
        if executable && writable {
            signals.push(Signal {
                kind: "writable_code",
                detail: format!(
                    "Section {} is both writable and executable, as unpacking stubs require",
                    display(&s.name)
                ),
            });
        }
        if s.raw_size == 0 && s.virtual_size >= 64 * 1024 && executable {
            signals.push(Signal {kind: "virtual_code_section", detail: format!("Section {} has {} virtual bytes but no file data; code is produced at run time", display(&s.name), s.virtual_size)});
        }
    }
    if let Some(entry) = image.sections.iter().position(|s| {
        image.entry >= s.virtual_address
            && image.entry < s.virtual_address + s.virtual_size.max(s.raw_size as u32)
    }) && image.entry != 0
        && entry + 1 == image.sections.len()
        && image.sections.len() > 1
    {
        signals.push(Signal {
            kind: "entry_in_last_section",
            detail: format!(
                "The native entry point is in the last section ({}), a common packer stub layout",
                display(&image.sections[entry].name)
            ),
        });
    }
    if image.end_of_sections > 0 && image.end_of_sections < bytes.len() {
        let overlay = &bytes[image.end_of_sections..];
        let e = entropy(&overlay[..overlay.len().min(1024 * 1024)]);
        signals.push(Signal {kind: "overlay", detail: format!("{} bytes follow the last section (entropy {e:.2}); installers, bundles and some packers store payloads there", overlay.len())});
    }
    if let Some(a) = assembly {
        managed(a, &mut detections, &mut signals);
    } else if high > 0 && detections.is_empty() {
        add(
            &mut detections,
            "Unknown packer",
            "packer",
            "heuristic",
            format!("{high} high-entropy sections in an image without CLR metadata"),
        );
    }
    // Only signature-level findings name a verdict; heuristics alone stay "suspicious".
    let signed = |kinds: &[&str]| {
        detections
            .iter()
            .any(|d| d.confidence == "signature" && kinds.contains(&d.kind))
    };
    let verdict = if signed(&["packer", "bundler"]) {
        "packed"
    } else if signed(&["protector", "virtualizer"]) {
        "protected"
    } else if signed(&["obfuscator"]) {
        "obfuscated"
    } else if !detections.is_empty() || !signals.is_empty() {
        "suspicious"
    } else {
        "no_findings"
    };
    Ok(DetectionReport {
        managed: assembly.is_some(),
        verdict,
        detections,
        signals,
    })
}
fn display(name: &str) -> String {
    if name.is_empty() {
        "(unnamed)".to_owned()
    } else {
        name.to_owned()
    }
}
fn managed(a: &Assembly, detections: &mut Vec<Detection>, signals: &mut Vec<Signal>) {
    // Attribute markers are already validated by the metadata inspection.
    for e in &a.info.obfuscation.evidence {
        if let Some(tool) = e.tool {
            let kind = if tool == "ConfuserEx" || tool == "Confuser family" {
                "protector"
            } else {
                "obfuscator"
            };
            add(detections, tool, kind, "signature", e.detail.clone());
        } else {
            signals.push(Signal {
                kind: e.kind,
                detail: e.detail.clone(),
            });
        }
    }
    for n in a.nodes.iter().filter(|n| n.token >> 24 == 2) {
        for (tool, kind, marker) in TYPE_SIGNATURES {
            let matched = if marker.ends_with('.') {
                n.full_name.starts_with(marker)
            } else {
                n.full_name == *marker || n.name == *marker
            };
            if matched {
                add(
                    detections,
                    tool,
                    kind,
                    "heuristic",
                    format!("Defines a type named {} (0x{:08X})", n.full_name, n.token),
                );
            }
        }
    }
    let m = a.metadata();
    for (i, r) in m.manifest_resources.iter().enumerate() {
        let Ok(name) = a.string(r.name) else { continue };
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("costura.")
            && (lower.ends_with(".compressed") || lower.ends_with(".zip"))
        {
            add(
                detections,
                "Costura.Fody",
                "bundler",
                "signature",
                format!("Embedded assembly resource {name}"),
            );
        }
        if r.implementation.row != 0 {
            continue;
        }
        if let Ok(data) = crate::export::resource(a, 0x28000001 + i as u32)
            && data.len() >= 64 * 1024
        {
            let e = entropy(&data);
            if e >= 7.5 {
                signals.push(Signal {kind: "encrypted_resource", detail: format!("Resource {name} is {} bytes with entropy {e:.2}; packers often store the real assembly encrypted in a resource", data.len())});
            }
            if data.starts_with(b"MZ") {
                signals.push(Signal {
                    kind: "embedded_pe",
                    detail: format!("Resource {name} contains an unencrypted PE image"),
                });
            }
        }
    }
    // Encrypted or anti-tampered method bodies fail to decode as CIL.
    let mut invalid = 0;
    let mut checked = 0;
    let mut first = None;
    for (index, row) in m.method_defs.iter().enumerate().take(20_000) {
        if row.rva == 0 || row.impl_flags & 3 != 0 {
            continue;
        }
        checked += 1;
        let token = 0x06000001 + index as u32;
        if a.body(token).is_err() {
            invalid += 1;
            first.get_or_insert(token);
        }
    }
    if invalid > 0 {
        let detail = format!(
            "{invalid} of {checked} method bodies are not valid CIL (first 0x{:08X}); bodies decrypted at run time (anti-tamper) look like this",
            first.unwrap_or(0)
        );
        if invalid * 4 >= checked {
            add(
                detections,
                "Encrypted method bodies",
                "protector",
                "heuristic",
                detail,
            );
        } else {
            signals.push(Signal {
                kind: "invalid_method_bodies",
                detail,
            });
        }
    }
    let module_cctor = a
        .nodes
        .iter()
        .any(|n| n.parent == 0x02000001 && n.name == ".cctor");
    let types = m.type_defs.len();
    let resource_bytes: usize = (0..m.manifest_resources.len())
        .filter_map(|i| crate::export::resource(a, 0x28000001 + i as u32).ok())
        .map(|d| d.len())
        .sum();
    if module_cctor {
        signals.push(Signal {kind: "module_initializer", detail: "<Module> has a static constructor, which runs before any other code; protectors use it for decryption and anti-tamper setup, though compilers also emit module initializers".into()});
    }
    if types <= 4 && resource_bytes >= 128 * 1024 && module_cctor {
        add(
            detections,
            "Managed loader stub",
            "packer",
            "heuristic",
            format!(
                "Only {types} types but {resource_bytes} bytes of resources and a module initializer; the real program is likely loaded from a resource"
            ),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn image(sections: &[(&str, u32, &[u8])], overlay: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x86..0x88].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        b[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes());
        let mut raw = 0x400u32;
        let mut va = 0x1000u32;
        let mut data = vec![];
        for (i, (name, flags, content)) in sections.iter().enumerate() {
            let h = 0x98 + 0xe0 + i * 40;
            b[h..h + name.len()].copy_from_slice(name.as_bytes());
            b[h + 8..h + 12].copy_from_slice(&(content.len() as u32).to_le_bytes());
            b[h + 12..h + 16].copy_from_slice(&va.to_le_bytes());
            b[h + 16..h + 20].copy_from_slice(&(content.len() as u32).to_le_bytes());
            b[h + 20..h + 24].copy_from_slice(&raw.to_le_bytes());
            b[h + 36..h + 40].copy_from_slice(&flags.to_le_bytes());
            if i + 1 == sections.len() {
                b[0x98 + 16..0x98 + 20].copy_from_slice(&va.to_le_bytes());
            }
            raw += content.len() as u32;
            va += 0x10000;
            data.extend_from_slice(content);
        }
        b.extend(data);
        b.extend_from_slice(overlay);
        b
    }
    fn noise(n: usize) -> Vec<u8> {
        let mut x = 0x2545f491u32;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }
    #[test]
    fn native_section_signatures_and_generic_signals() {
        let packed = noise(8192);
        let bytes = image(
            &[("UPX0", 0xe000_0080, &[]), ("UPX1", 0xe000_0040, &packed)],
            b"tail",
        );
        let report = scan(&bytes, None).unwrap();
        assert_eq!(report.verdict, "packed");
        assert_eq!(report.detections[0].name, "UPX");
        assert_eq!(report.detections[0].confidence, "signature");
        let kinds: Vec<_> = report.signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&"high_entropy_section"));
        assert!(kinds.contains(&"writable_code"));
        assert!(kinds.contains(&"entry_in_last_section"));
        assert!(kinds.contains(&"overlay"));
    }
    #[test]
    fn unknown_high_entropy_native_image_is_only_a_heuristic() {
        let bytes = image(
            &[
                (".text", 0x6000_0020, &[0x90; 8192]),
                (".data", 0xc000_0040, &noise(8192)),
            ],
            &[],
        );
        let report = scan(&bytes, None).unwrap();
        assert_eq!(report.detections[0].name, "Unknown packer");
        assert_eq!(report.detections[0].confidence, "heuristic");
        assert_eq!(report.verdict, "suspicious");
        let plain = image(&[(".text", 0x6000_0020, &[0x90; 8192])], &[]);
        assert_eq!(scan(&plain, None).unwrap().verdict, "no_findings");
    }
    #[test]
    fn truncated_headers_do_not_panic() {
        let bytes = image(&[(".text", 0x6000_0020, &[0x90; 16])], &[]);
        for len in 0..bytes.len() {
            let _ = scan(&bytes[..len], None);
        }
    }
}
