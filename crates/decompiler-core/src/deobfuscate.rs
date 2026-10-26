//! Incremental generic transformations and recognized static resource-string profiles.
//! No executable unpacking, token remapping, initializer execution or member deletion.
pub(crate) mod eaz;
pub(crate) mod proxy;
pub(crate) mod rename;
use crate::{
    assembly::Assembly,
    cil_encode::{self, EditableInstruction},
    edit::{self, ValidatedEdit},
    emulate::{self, Budget, Evaluator, Value},
    error::{Error, ErrorCode, Result},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub constants: bool,
    pub branches: bool,
    pub strings: bool,
    #[serde(default)]
    pub resource_strings: bool,
    #[serde(default)]
    pub rename: bool,
    #[serde(default)]
    pub proxies: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            constants: true,
            branches: true,
            strings: false,
            resource_strings: false,
            rename: false,
            proxies: false,
        }
    }
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Counts {
    pub constants: usize,
    pub branches: usize,
    pub strings: usize,
    pub unreachable_instructions: usize,
    pub renamed: usize,
    pub proxies: usize,
}
impl Counts {
    fn changed(&self) -> bool {
        self.constants + self.branches + self.strings + self.proxies > 0
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MethodReport {
    pub token: u32,
    pub status: String,
    pub counts: Counts,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub outcome: String,
    pub options: Options,
    pub methods: Vec<MethodReport>,
    pub changed_methods: usize,
    pub counts: Counts,
    pub skipped_methods: usize,
    pub unresolved_calls: usize,
    pub evaluation_steps: usize,
    pub notices: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub job: u32,
    pub completed: usize,
    pub total: usize,
    pub changed_methods: usize,
    pub phase: String,
    pub done: bool,
}
pub struct Job {
    pub id: u32,
    pub assembly: u32,
    pub revision: u32,
    cursor: usize,
    total: usize,
    instructions: usize,
    retained: usize,
    edits: BTreeMap<u32, ValidatedEdit>,
    resource: Option<eaz::Profile>,
    /// Same-size replacement of any static `string(int)` decoder the evaluator resolves,
    /// used where the general IL writer cannot rewrite whole methods.
    literal_strings: bool,
    literals: Vec<crate::pe_write::LiteralPatch>,
    pub report: Report,
    pub output: Option<Vec<u8>>,
    pub opened: Option<u32>,
}
pub fn cancelled() -> Error {
    Error::new(
        ErrorCode::Cancelled,
        "Deobfuscation was cancelled, superseded, or invalidated by a workspace change",
    )
}
impl Job {
    pub fn new(
        id: u32,
        assembly: u32,
        revision: u32,
        a: &Assembly,
        mut options: Options,
    ) -> Result<Self> {
        let generic = options.constants || options.branches || options.strings;
        if !generic && !options.resource_strings && !options.rename && !options.proxies {
            return Err(edit::unsupported("Select at least one transformation"));
        }
        let mut resource = if options.resource_strings {
            eaz::Profile::detect(a)?
        } else {
            None
        };
        let mut skipped_notice = None;
        // A cleaned copy still contains the decoder; with no call sites left the
        // generic transforms run instead of an empty resource pass.
        if let Some(profile) = &resource {
            let call = [&[0x28u8][..], &profile.helper.to_le_bytes()].concat();
            if !a.bytes.windows(5).any(|w| w == call.as_slice()) {
                resource = None;
                skipped_notice = Some("The recognized string decoder has no remaining call sites; generic transforms were used instead.".to_string());
            }
        }
        let methods = if resource.is_some() || options.proxies || !generic {
            16_384
        } else {
            4096
        };
        if a.bytes.len() > 16 * 1024 * 1024 || a.metadata().method_defs.len() > methods {
            return Err(Error::limit(format!(
                "This deobfuscation supports at most 16 MiB and {methods} methods"
            )));
        }
        if a.info.reference_assembly {
            return Err(edit::unsupported(
                "Reference assemblies contain API stubs, not implementations to clean",
            ));
        }
        // Proxy unwrapping and resource strings only need the same-size (literal) writer.
        if resource.is_some() || options.proxies || !generic {
            crate::pe_write::literal_capability(a)?;
        }
        let mut literal_strings = false;
        if generic && let Err(e) = crate::pe_write::capability(a) {
            // Renaming, proxy unwrapping and same-size string replacement survive a
            // layout the general IL writer cannot handle (Field-RVA arrays, for one);
            // only the whole-method IL transforms are dropped.
            literal_strings = options.strings;
            if !options.rename && !options.proxies && !literal_strings {
                return Err(e);
            }
            (options.constants, options.branches) = (false, false);
            options.strings = literal_strings;
            if literal_strings {
                crate::pe_write::literal_capability(a)?;
            }
            skipped_notice = Some(format!("Generic IL transforms skipped: {}", e.detail));
        }
        let mut notices=vec!["Generic transforms only. No obfuscator family is claimed as fully supported.".into(),"Declarations, names, tokens, resources and initializers are retained. Unsupported methods/calls remain unchanged.".into(),"Changed methods pass the strict writer checks. This is not a proof of whole-program behavioral equivalence; review the output before use.".into()];
        if resource.is_some() {
            notices = vec!["Recognized legacy Eazfuscator-compatible resource-string layout. This pass replaces supported string(int) calls only; run generic transforms separately on the resulting copy if needed.".into(), "No CLR, reflection or initializer execution. Resource records and bounded integer key expressions are read statically.".into(), "Instruction offsets, exception regions, metadata tokens, names, Field-RVA data, resources and helper definitions are preserved. Original names and complete de4dot feature parity are not recovered.".into(), "Replacing decoder calls removes their initialization/guard/cache side effects at those call sites. This is an analysis copy, not a whole-program equivalence guarantee.".into()];
        } else if options.resource_strings {
            notices.push("No compatible legacy resource-string layout found. Unsupported protection families are not executed or guessed.".into());
        }
        if let Some(n) = skipped_notice {
            notices.push(n);
        }
        if options.rename {
            notices.push("Obfuscated type, member, parameter and generic-parameter names are replaced with synthetic names. Overrides keep matching names; reflection, serialization or resource lookups by string may no longer match.".into());
        }
        if options.proxies {
            notices.push("Proxy/indirect call unwrapping redirects calls to pure forwarder methods straight to their target, preserving instruction offsets and exception regions. Forwarder definitions are kept.".into());
        }
        if options.strings && resource.is_none() {
            notices.push("Pure string evaluation interprets a bounded CIL subset in ILens, with no CLR execution. Only literal or explicitly interned results are materialized; allocation and interning timing can still differ.".into());
        }
        Ok(Self {
            id,
            assembly,
            revision,
            cursor: 0,
            total: a.metadata().method_defs.len(),
            instructions: 0,
            retained: 0,
            resource,
            literal_strings,
            literals: vec![],
            edits: BTreeMap::new(),
            output: None,
            opened: None,
            report: Report {
                outcome: "pending".into(),
                options,
                methods: vec![],
                changed_methods: 0,
                counts: Counts::default(),
                skipped_methods: 0,
                unresolved_calls: 0,
                evaluation_steps: 0,
                notices,
            },
        })
    }
    pub fn progress(&self) -> Progress {
        Progress {
            job: self.id,
            completed: self.cursor,
            total: self.total,
            changed_methods: self.report.changed_methods,
            phase: if self.cursor == self.total {
                "Ready to validate output"
            } else {
                "Inspecting methods"
            }
            .into(),
            done: self.cursor == self.total,
        }
    }
    pub fn step(&mut self, a: &Assembly) -> Result<Progress> {
        if self.cursor >= self.total {
            return Ok(self.progress());
        }
        let token = 0x06000001 + self.cursor as u32;
        let mut result = MethodReport {
            token,
            status: "unchanged".into(),
            counts: Counts::default(),
            reasons: vec![],
        };
        match a.body(token) {
            Err(e) => {
                result.status = "skipped".into();
                result.reasons.push(e.detail.chars().take(200).collect());
            }
            // Abstract, interface, runtime and P/Invoke members have nothing to clean.
            Ok(None) => {}
            Ok(Some(body)) => {
                self.instructions += body.instructions.len();
                // The 200k budget guards the per-method generic rewrite path. The
                // same-size passes (resource strings, proxy unwrapping, rename) only
                // read bodies, so they use the larger whole-image budget.
                let generic_rows = self.report.options.constants
                    || self.report.options.branches
                    || self.report.options.strings;
                let max_instructions =
                    if self.resource.is_some() || self.literal_strings || !generic_rows {
                        2_000_000
                    } else {
                        200_000
                    };
                if self.instructions > max_instructions {
                    return Err(Error::limit(format!(
                        "Deobfuscation input exceeds {max_instructions} decoded instructions"
                    )));
                }
                if let Some(profile) = &mut self.resource {
                    let boundaries = crate::pe_write::literal_boundaries(&body);
                    for (index, call) in body.instructions.iter().enumerate() {
                        if call.opcode != 0x28 || call.token() != Some(profile.helper) {
                            continue;
                        }
                        if !crate::pe_write::literal_site(&body, index, &boundaries, false) {
                            self.report.unresolved_calls += 1;
                            note(
                                &mut result.reasons,
                                "String call is not preceded by one literal in the same branch/exception region",
                            );
                            continue;
                        }
                        let argument = &body.instructions[index - 1];
                        match profile.decode(
                            eaz::constant(argument)
                                .ok_or_else(|| Error::cil("Expected literal argument"))?,
                        ) {
                            Ok(text) => {
                                if self.retained + text.len() + 64 > 8 * 1024 * 1024
                                    || self.literals.len() >= 100_000
                                {
                                    self.report.unresolved_calls += 1;
                                    note(
                                        &mut result.reasons,
                                        "Resource-string patches reached the 8 MiB / 100,000 call limit; call retained",
                                    );
                                    continue;
                                }
                                self.retained += text.len() + 64;
                                self.literals.push(crate::pe_write::LiteralPatch {
                                    method: token,
                                    argument_offset: argument.offset,
                                    call_offset: call.offset,
                                    helper: profile.helper,
                                    text,
                                });
                                result.counts.strings += 1;
                                result.status = "changed".into();
                            }
                            Err(e) => {
                                self.report.unresolved_calls += 1;
                                note(&mut result.reasons, &e.detail);
                            }
                        }
                    }
                    if result.status == "changed" {
                        self.report.changed_methods += 1;
                    }
                } else if self.literal_strings {
                    self.decode_literals(a, token, &body, &mut result)?;
                    if result.status == "changed" {
                        self.report.changed_methods += 1;
                    }
                } else if !(self.report.options.constants
                    || self.report.options.branches
                    || self.report.options.strings
                    || self.report.options.proxies)
                {
                    // Rename-only job: names are handled when the output is written.
                } else if body.instructions.len() > emulate::MAX_INSTRUCTIONS
                    || !body.exceptions.is_empty()
                {
                    result.status = "skipped".into();
                    result.reasons.push("Initial transforms require at most 2,000 instructions and no exception regions".into());
                } else {
                    let mut rows = cil_encode::from_body(&body);
                    let targets: HashSet<String> = body
                        .instructions
                        .iter()
                        .flat_map(|i| i.targets())
                        .map(|n| format!("IL_{n:04X}"))
                        .collect();
                    let mut evaluator = Evaluator::new(a, Budget::default());
                    let mut attempts = 0;
                    for index in 0..rows.len() {
                        if self.report.options.constants {
                            fold_constant(&mut rows, index, &targets, &mut result.counts);
                        }
                        if self.report.options.branches {
                            fold_branch(&mut rows, index, &targets, &mut result.counts);
                        }
                        if self.report.options.strings && rows[index].opcode == "call" {
                            let Some(target) =
                                parse_token(&rows[index].operand).filter(|t| t >> 24 == 6)
                            else {
                                continue;
                            };
                            let Ok(method) = a.method_ref(target) else {
                                continue;
                            };
                            if method.signature.return_type
                                != crate::signature::Type::primitive("string")
                                || method.signature.has_this
                            {
                                continue;
                            }
                            if attempts >= 16 {
                                note(
                                    &mut result.reasons,
                                    "At most 16 string-helper candidates are evaluated per method",
                                );
                                self.report.unresolved_calls += 1;
                                continue;
                            }
                            attempts += 1;
                            let n = method.signature.parameters.len();
                            let sources = previous(&rows, index, n, &targets);
                            let evaluated = (|| {
                                let sources=sources.as_ref().filter(|s|n<=8&&s.len()==n).ok_or_else(||emulate::Stop("String helper arguments are not consecutive literals in one basic block".into()))?;
                                let mut args = Vec::new();
                                for &source in sources {
                                    let r = &rows[source];
                                    args.push(if let Some(value)=emulate::constant(&r.opcode,&r.operand){Value::I32(value)}else if r.opcode=="ldstr"{evaluator.literal(parse_token(&r.operand).ok_or_else(||emulate::Stop("Expected original user-string token".into()))?)?}else{return Err(emulate::Stop("String helper requires literal int32/string arguments".into()));});
                                }
                                let result = evaluator.call(target, args)?;
                                match result{
                                    Value::String(units,true)=>String::from_utf16(&units).map_err(|_|emulate::Stop("Unpaired UTF-16 surrogate cannot be emitted without loss".into())),
                                    Value::String(_,false)=>Err(emulate::Stop("Non-interned result retained to avoid changing string reference identity".into())),
                                    _=>Err(emulate::Stop("Helper did not return a string".into())),
                                }
                            })();
                            match evaluated {
                                Ok(text) => {
                                    for source in sources.unwrap_or_default() {
                                        nop(&mut rows[source]);
                                    }
                                    rows[index].opcode = "ldstr".into();
                                    rows[index].operand = serde_json::to_string(&text)
                                        .map_err(|e| Error::metadata(e.to_string()))?;
                                    result.counts.strings += 1;
                                }
                                Err(e) => {
                                    self.report.unresolved_calls += 1;
                                    note(&mut result.reasons, &e.0);
                                }
                            }
                        }
                    }
                    self.report.evaluation_steps += evaluator.budget.steps;
                    if self.report.evaluation_steps > 1_000_000 {
                        return Err(Error::limit(
                            "Deobfuscation job exceeded one million evaluation steps",
                        ));
                    }
                    if self.report.options.branches {
                        thread_branches(&mut rows, &mut result.counts);
                    }
                    if result.counts.changed() {
                        // Neither the transformation nor concrete evaluation bypasses the writer verifier.
                        let validated = edit::verify(a, token, &body).and_then(|_| {
                            // Only instruction blocks made unreachable by proven local
                            // transformations are removed. No declarations are deleted.
                            result.counts.unreachable_instructions = prune_unreachable(&mut rows)?;
                            edit::validate_types(a, token, &rows, None, None)
                        });
                        match validated {
                            Ok(edit) => {
                                if self.edits.len() >= 256 {
                                    return Err(Error::limit(
                                        "At most 256 changed methods per deobfuscation job",
                                    ));
                                }
                                self.retained += edit.code.len()
                                    + edit
                                        .instructions
                                        .iter()
                                        .map(|i| {
                                            128 + i.id.len() + i.opcode.len() + i.operand.len()
                                        })
                                        .sum::<usize>()
                                    + edit.strings.iter().map(String::len).sum::<usize>();
                                if self.retained > 8 * 1024 * 1024 {
                                    return Err(Error::limit(
                                        "Pending deobfuscation changes exceed 8 MiB",
                                    ));
                                }
                                self.edits.insert(token, edit);
                                result.status = "changed".into();
                            }
                            Err(e) => {
                                result.status = "skipped".into();
                                result.counts = Counts::default();
                                note(
                                    &mut result.reasons,
                                    &format!("Strict validation: {}", e.detail),
                                );
                            }
                        }
                    }
                }
            }
        }
        self.report.counts.constants += result.counts.constants;
        self.report.counts.branches += result.counts.branches;
        self.report.counts.strings += result.counts.strings;
        self.report.counts.unreachable_instructions += result.counts.unreachable_instructions;
        self.report.skipped_methods += usize::from(result.status == "skipped");
        self.report.methods.push(result);
        self.cursor += 1;
        if self.resource.is_none() && !self.literal_strings {
            self.report.changed_methods = self.edits.len();
        }
        Ok(self.progress())
    }
    pub fn finish(&mut self, a: &Assembly) -> Result<Report> {
        if self.cursor != self.total {
            return Err(edit::unsupported(
                "Complete all deobfuscation steps before finishing",
            ));
        }
        if self.output.is_none() {
            // The existing writer independently revalidates edits and reparses its output.
            let bytes = if self.resource.is_some() || self.literal_strings {
                crate::pe_write::write_literals(a, &self.literals)?
            } else if self.edits.is_empty() {
                a.bytes.clone()
            } else {
                crate::pe_write::write(a, &self.edits)?
            };
            // Proxy unwrapping is a same-size patch applied on top of whatever the
            // string/IL passes produced; it preserves offsets and exception regions.
            let bytes = if self.report.options.proxies {
                self.unwrap_proxies(a, bytes)?
            } else {
                bytes
            };
            let bytes = if self.report.options.rename {
                self.rename(a, bytes)?
            } else {
                bytes
            };
            let unchanged = self.edits.is_empty()
                && self.literals.is_empty()
                && self.report.counts.proxies == 0
                && self.report.counts.renamed == 0;
            self.report.outcome = if unchanged {
                "no_changes"
            } else if self.report.skipped_methods > 0 || self.report.unresolved_calls > 0 {
                "partial"
            } else {
                "changed"
            }
            .into();
            self.report.notices.push(if unchanged{"No change: output is byte-for-byte identical to the input."}else{"Output reparsed successfully. Each replacement passed its writer validation; original metadata/resources and unchanged bytes were checked. Debug records were cleared."}.into());
            self.output = Some(bytes);
            self.edits.clear();
            self.literals.clear();
            self.resource = None;
        }
        Ok(self.report.clone())
    }
}
impl Job {
    /// Replaces `ldc.i4 K; call static string Helper(int)` with the evaluated literal,
    /// in place and without moving any instruction. The evaluator decides what is
    /// resolvable; this only chooses the sites it is offered.
    fn decode_literals(
        &mut self,
        a: &Assembly,
        token: u32,
        body: &crate::cil::MethodBody,
        result: &mut MethodReport,
    ) -> Result<()> {
        let boundaries = crate::pe_write::literal_boundaries(body);
        let mut evaluator = Evaluator::new(a, Budget::default());
        let mut attempts = 0;
        for (index, call) in body.instructions.iter().enumerate() {
            if call.opcode != 0x28 {
                continue;
            }
            let Some(helper) = call.token().filter(|t| t >> 24 == 6 && *t != token) else {
                continue;
            };
            let Ok(method) = a.method_ref(helper) else {
                continue;
            };
            let takes_string =
                method.signature.parameters == [crate::signature::Type::primitive("string")];
            if method.signature.has_this
                || method.signature.generic_count != 0
                || method.signature.return_type != crate::signature::Type::primitive("string")
                || !(takes_string
                    || method.signature.parameters == [crate::signature::Type::primitive("int")])
                || !crate::pe_write::literal_site(body, index, &boundaries, takes_string)
            {
                continue;
            }
            if attempts >= 16 {
                note(
                    &mut result.reasons,
                    "At most 16 string-helper candidates are evaluated per method",
                );
                self.report.unresolved_calls += 1;
                continue;
            }
            attempts += 1;
            let argument = &body.instructions[index - 1];
            let value = if takes_string {
                evaluator.literal(argument.token().unwrap_or(0))
            } else {
                Ok(Value::I32(eaz::constant(argument).ok_or_else(|| {
                    Error::cil("Expected a literal decoder argument")
                })?))
            };
            let decoded = value
                .and_then(|value| evaluator.call(helper, vec![value]))
                .and_then(|value| match value {
                    Value::String(units, true) => String::from_utf16(&units)
                        .map_err(|_| emulate::Stop("Unpaired UTF-16 surrogate".into())),
                    Value::String(_, false) => Err(emulate::Stop(
                        "Non-interned result retained to avoid changing string reference identity"
                            .into(),
                    )),
                    _ => Err(emulate::Stop("Helper did not return a string".into())),
                });
            match decoded {
                Ok(text) => {
                    if self.retained + text.len() + 64 > 8 * 1024 * 1024
                        || self.literals.len() >= 100_000
                    {
                        self.report.unresolved_calls += 1;
                        note(
                            &mut result.reasons,
                            "String patches reached the 8 MiB / 100,000 call limit; call retained",
                        );
                        continue;
                    }
                    self.retained += text.len() + 64;
                    self.literals.push(crate::pe_write::LiteralPatch {
                        method: token,
                        argument_offset: argument.offset,
                        call_offset: call.offset,
                        helper,
                        text,
                    });
                    result.counts.strings += 1;
                    result.status = "changed".into();
                }
                Err(e) => {
                    self.report.unresolved_calls += 1;
                    note(&mut result.reasons, &e.0);
                }
            }
        }
        self.report.evaluation_steps += evaluator.budget.steps;
        if self.report.evaluation_steps > 1_000_000 {
            return Err(Error::limit(
                "Deobfuscation job exceeded one million evaluation steps",
            ));
        }
        Ok(())
    }
    /// Unwraps proxy/forwarder calls on top of the already written bytes. Forwarders
    /// are re-detected from the current image (tokens are stable) and call sites are
    /// rewritten in place. A failure keeps the input bytes and is reported.
    fn unwrap_proxies(&mut self, a: &Assembly, bytes: Vec<u8>) -> Result<Vec<u8>> {
        let owned;
        let base = if bytes == a.bytes {
            a
        } else {
            owned = Assembly::load(bytes.clone())?;
            &owned
        };
        match crate::pe_write::write_proxies(base) {
            Ok((output, count)) => {
                self.report.counts.proxies = count;
                if count > 0 {
                    self.report
                        .notices
                        .push(format!("Unwrapped {count} proxy/indirect call sites."));
                }
                Ok(output)
            }
            Err(e) => {
                self.report
                    .notices
                    .push(format!("Proxy unwrapping skipped: {}", e.detail));
                Ok(bytes)
            }
        }
    }
    /// Renames on top of the already validated output; a rename failure keeps that
    /// output and is reported, it does not discard the other transformations.
    fn rename(&mut self, a: &Assembly, bytes: Vec<u8>) -> Result<Vec<u8>> {
        let owned;
        let base = if bytes == a.bytes {
            a
        } else {
            owned = Assembly::load(bytes.clone())?;
            &owned
        };
        let renamed = rename::plan(base).and_then(|plan| {
            let output = crate::pe_write::write_names(base, &plan.cells)?;
            Ok((plan, output))
        });
        match renamed {
            Ok((plan, output)) => {
                self.report.counts.renamed = plan.cells.len();
                if !plan.cells.is_empty() {
                    self.report.notices.push(format!(
                        "Renamed {} types and {} members ({} name cells).",
                        plan.types,
                        plan.members,
                        plan.cells.len()
                    ));
                }
                if plan.unresolved_references > 0 {
                    self.report.notices.push(format!("{} member references into renamed types could not be matched and keep their original names.", plan.unresolved_references));
                }
                Ok(output)
            }
            Err(e) => {
                self.report
                    .notices
                    .push(format!("Symbol renaming skipped: {}", e.detail));
                Ok(bytes)
            }
        }
    }
}
fn note(reasons: &mut Vec<String>, text: &str) {
    let s: String = text.chars().take(220).collect();
    if reasons.len() < 8 && !reasons.contains(&s) {
        reasons.push(s);
    }
}
fn prune_unreachable(rows: &mut Vec<EditableInstruction>) -> Result<usize> {
    use crate::cil::Flow;
    let labels: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let ops: HashMap<_, _> = (0..=255)
        .chain(0xfe00..=0xfeff)
        .filter_map(crate::opcodes::opcode)
        .map(|op| (op.name, op.flow))
        .collect();
    let mut pending = vec![0];
    let mut visited = vec![false; rows.len()];
    while let Some(i) = pending.pop() {
        if i >= rows.len() || visited[i] {
            continue;
        }
        visited[i] = true;
        let row = &rows[i];
        let flow = ops
            .get(row.opcode.as_str())
            .ok_or_else(|| Error::cil("Unknown transformation opcode"))?;
        if matches!(flow, Flow::Branch | Flow::CondBranch) && !row.operand.trim().is_empty() {
            for target in row.operand.split(',').map(str::trim) {
                pending.push(
                    *labels
                        .get(target)
                        .ok_or_else(|| Error::cil("Unknown transformation branch label"))?,
                );
            }
        }
        if !matches!(flow, Flow::Branch | Flow::Return | Flow::Throw) {
            pending.push(i + 1);
        }
    }
    let before = rows.len();
    let mut index = 0;
    rows.retain(|_| {
        let keep = visited[index];
        index += 1;
        keep
    });
    Ok(before - rows.len())
}
fn parse_token(text: &str) -> Option<u32> {
    u32::from_str_radix(text.strip_prefix("0x")?, 16).ok()
}
fn nop(row: &mut EditableInstruction) {
    row.opcode = "nop".into();
    row.operand.clear();
}
fn previous(
    rows: &[EditableInstruction],
    end: usize,
    count: usize,
    targets: &HashSet<String>,
) -> Option<Vec<usize>> {
    if count == 0 {
        return Some(vec![]);
    }
    let mut found = Vec::new();
    let mut i = end;
    while i > 0 && found.len() < count {
        if targets.contains(&rows[i].id) {
            return None;
        }
        i -= 1;
        if rows[i].opcode != "nop" {
            found.push(i);
        }
    }
    if found.len() != count {
        return None;
    }
    found.reverse();
    Some(found)
}
fn fold_constant(
    rows: &mut [EditableInstruction],
    i: usize,
    targets: &HashSet<String>,
    counts: &mut Counts,
) {
    let Some(sources) = previous(rows, i, 2, targets) else {
        return;
    };
    let (Some(a), Some(b)) = (
        emulate::constant(&rows[sources[0]].opcode, &rows[sources[0]].operand),
        emulate::constant(&rows[sources[1]].opcode, &rows[sources[1]].operand),
    ) else {
        return;
    };
    let Some(n) = emulate::binary(&rows[i].opcode, a, b) else {
        return;
    };
    for source in sources {
        nop(&mut rows[source]);
    }
    rows[i].opcode = "ldc.i4".into();
    rows[i].operand = n.to_string();
    counts.constants += 1;
}
fn fold_branch(
    rows: &mut [EditableInstruction],
    i: usize,
    targets: &HashSet<String>,
    counts: &mut Counts,
) {
    let op = rows[i].opcode.clone();
    let unary = matches!(op.as_str(), "brtrue" | "brtrue.s" | "brfalse" | "brfalse.s");
    if !unary && emulate::condition(&op, 0, 0).is_none() {
        return;
    }
    let Some(sources) = previous(rows, i, if unary { 1 } else { 2 }, targets) else {
        return;
    };
    let values: Option<Vec<i32>> = sources
        .iter()
        .map(|s| emulate::constant(&rows[*s].opcode, &rows[*s].operand))
        .collect();
    let Some(v) = values else { return };
    let taken = if unary {
        (v[0] != 0) == op.starts_with("brtrue")
    } else {
        emulate::condition(&op, v[0], v[1]).unwrap_or(false)
    };
    for source in sources {
        nop(&mut rows[source]);
    }
    if taken {
        rows[i].opcode = "br".into();
    } else {
        nop(&mut rows[i]);
    }
    counts.branches += 1;
}
fn thread_branches(rows: &mut [EditableInstruction], counts: &mut Counts) {
    let labels: HashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();
    for i in 0..rows.len() {
        if !matches!(rows[i].opcode.as_str(), "br" | "br.s") {
            continue;
        }
        let original = rows[i].operand.clone();
        let mut target = original.clone();
        let mut visited = HashSet::from([i]);
        for _ in 0..64 {
            let Some(&at) = labels.get(&target) else {
                break;
            };
            if !visited.insert(at) {
                target = original.clone();
                break;
            }
            if !matches!(rows[at].opcode.as_str(), "br" | "br.s") {
                break;
            }
            target = rows[at].operand.clone();
        }
        if target != original {
            rows[i].operand = target;
            counts.branches += 1;
        }
    }
}
