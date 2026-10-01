//! Paged, static relationship analysis across explicitly loaded assemblies.
use crate::{
    assembly::coded,
    error::{Error, Result},
    session::Session,
};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct Hit {
    pub assembly: u32,
    pub token: u32,
    pub name: String,
    pub offset: Option<u32>,
    pub relation: &'static str,
}
#[derive(Serialize)]
pub struct Page {
    pub hits: Vec<Hit>,
    pub next: usize,
    pub total: usize,
    pub done: bool,
    pub skipped: usize,
    pub truncated: bool,
}
impl Session {
    pub fn analyze_page(
        &self,
        id: u32,
        token: u32,
        source_id: u32,
        start: usize,
        outgoing: bool,
    ) -> Result<Page> {
        self.assembly(id)?.validate_token(token)?;
        let target = self.resolve_definition(id, token).unwrap_or((id, token));
        let source = self.assembly(source_id)?;
        let target_assembly = self.assembly(target.0)?;
        let mut targets = vec![target];
        if let Some(accessors) = target_assembly.semantics.get(&target.1) {
            targets.extend(accessors.iter().map(|(_, t)| (target.0, *t)));
        }
        let mut resolved = HashMap::new();
        let mut resolve = |t: u32| {
            *resolved.entry(t).or_insert_with(|| {
                self.resolve_definition(source_id, t)
                    .unwrap_or((source_id, t))
            })
        };
        let mut hits = Vec::new();
        let mut skipped = 0;
        let mut truncated = false;
        let total = if outgoing { 1 } else { source.nodes.len() };
        let end = start.saturating_add(32).min(total);
        if outgoing && (source_id != id || token >> 24 != 6) {
            return Err(Error::metadata(
                "Choose a method to inspect outgoing references",
            ));
        }
        for index in start.min(total)..end {
            let selected = if outgoing {
                token
            } else {
                source.nodes[index].token
            };
            if selected >> 24 == 2 && !outgoing && target.1 >> 24 == 2 {
                let row = &source.metadata().type_defs[(selected & 0xffffff) as usize - 1];
                if coded(row.extends) != 0 && resolve(coded(row.extends)) == target {
                    hits.push(Hit {
                        assembly: source_id,
                        token: selected,
                        name: source.resolve(selected),
                        offset: None,
                        relation: "extends",
                    });
                }
                for interface in source
                    .metadata()
                    .interface_impls
                    .iter()
                    .filter(|i| i.class == selected & 0xffffff)
                {
                    if resolve(coded(interface.interface)) == target {
                        hits.push(Hit {
                            assembly: source_id,
                            token: selected,
                            name: source.resolve(selected),
                            offset: None,
                            relation: "implements",
                        });
                    }
                }
            }
            if selected >> 24 != 6 {
                continue;
            }
            let body = match source.body(selected) {
                Ok(Some(body)) => body,
                Ok(None) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            for instruction in body.instructions {
                let Some(operand) = instruction.token() else {
                    continue;
                };
                if operand >> 24 == 0x70 {
                    continue;
                }
                let definition = resolve(operand);
                let owner_matches = target.1 >> 24 == 2
                    && self
                        .assembly(definition.0)?
                        .owners
                        .get(&definition.1)
                        .copied()
                        == Some(target.1)
                    && definition.0 == target.0;
                if !outgoing && !targets.contains(&definition) && !owner_matches {
                    continue;
                }
                let relation = match instruction.opcode {
                    0x28 | 0x6f => "calls",
                    0x73 => "creates",
                    0x7b | 0x7e => "reads",
                    0x7d | 0x80 => "writes",
                    0x7c | 0x7f => "address",
                    0xfe06 | 0xfe07 => "method_group",
                    _ => "references",
                };
                let (assembly, referenced) = if outgoing {
                    definition
                } else {
                    (source_id, selected)
                };
                hits.push(Hit {
                    assembly,
                    token: referenced,
                    name: self.assembly(assembly)?.resolve(referenced),
                    offset: Some(instruction.offset),
                    relation,
                });
                if hits.len() >= 512 {
                    truncated = true;
                    break;
                }
            }
            if truncated {
                return Ok(Page {
                    hits,
                    next: index + 1,
                    total,
                    done: index + 1 >= total,
                    skipped,
                    truncated,
                });
            }
        }
        Ok(Page {
            hits,
            next: end,
            total,
            done: end >= total,
            skipped,
            truncated,
        })
    }
}
