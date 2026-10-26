//! Managed assemblies carried inside another assembly's embedded resources.
//!
//! Packers and merge tools keep their real payload here. Finding it is a read: the
//! resource bytes are parsed as a PE/CLI image exactly like a file the user opened,
//! and nothing is executed, decompressed or unpacked. A resource that only looks like
//! an assembly, or one stored compressed, is reported as unrecognized rather than
//! guessed at.
use crate::{assembly::Assembly, error::Result};
use serde::Serialize;

/// 64 MiB of resource bytes and 256 reported payloads per assembly.
const MAX_SCANNED: usize = 64 * 1024 * 1024;
const MAX_FOUND: usize = 256;
const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Embedded {
    pub token: u32,
    /// The entry inside a `.resources` container, or -1 for the whole resource.
    pub entry: i32,
    pub name: String,
    pub identity: String,
    pub runtime: String,
    pub size: usize,
    pub methods: usize,
    pub types: usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct Payloads {
    pub assemblies: Vec<Embedded>,
    /// Resources that start like a PE image but could not be parsed as one; these are
    /// where a compressed or encrypted payload would show up.
    pub unrecognized: Vec<String>,
    pub truncated: bool,
}
fn describe(token: u32, entry: i32, name: String, bytes: &[u8]) -> Option<Embedded> {
    if bytes.len() < 0x40 || &bytes[..2] != b"MZ" || bytes.len() > MAX_PAYLOAD {
        return None;
    }
    let a = Assembly::load(bytes.to_vec()).ok()?;
    Some(Embedded {
        token,
        entry,
        name,
        identity: format!(
            "{}, Version={}, Culture={}, PublicKeyToken={}",
            a.info.identity.name,
            a.info.identity.version,
            a.info.identity.culture,
            a.info.identity.public_key_token
        ),
        runtime: a.info.runtime.clone(),
        size: bytes.len(),
        methods: a.metadata().method_defs.len(),
        types: a.metadata().type_defs.len(),
    })
}
/// Lists the managed assemblies this assembly carries in its embedded resources.
pub fn payloads(a: &Assembly) -> Result<Payloads> {
    let mut found = Payloads {
        assemblies: vec![],
        unrecognized: vec![],
        truncated: false,
    };
    let mut scanned = 0usize;
    for index in 0..a.metadata().manifest_resources.len() {
        let token = 0x28000001 + index as u32;
        let Ok(bytes) = crate::export::resource(a, token) else {
            continue;
        };
        scanned += bytes.len();
        if scanned > MAX_SCANNED || found.assemblies.len() >= MAX_FOUND {
            found.truncated = true;
            break;
        }
        let name = a
            .metadata()
            .manifest_resources
            .get(index)
            .and_then(|r| a.string(r.name).ok())
            .unwrap_or_default()
            .to_owned();
        if let Some(payload) = describe(token, -1, name.clone(), &bytes) {
            found.assemblies.push(payload);
            continue;
        }
        if bytes.starts_with(b"MZ") {
            found.unrecognized.push(name.clone());
            continue;
        }
        // A `.resources` container is read as data; its entries are never deserialized.
        let Ok(preview) = crate::resources::preview(&bytes) else {
            continue;
        };
        for entry in &preview.entries {
            if found.assemblies.len() >= MAX_FOUND {
                found.truncated = true;
                break;
            }
            let Ok(data) = crate::resources::entry_bytes(&bytes, entry.index) else {
                continue;
            };
            let label = format!("{name} › {}", entry.name);
            if let Some(payload) = describe(token, entry.index as i32, label.clone(), &data) {
                found.assemblies.push(payload);
            } else if data.starts_with(b"MZ") {
                found.unrecognized.push(label);
            }
        }
    }
    found.unrecognized.truncate(MAX_FOUND);
    Ok(found)
}
