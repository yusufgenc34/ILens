//! Source-level idiom recovery on rendered method bodies, as ILSpy/dnSpy present them:
//! dead stack copies, `?.`, `using`, `foreach` and unused catch variables. Each pass
//! rewrites only exact patterns whose variables are not referenced anywhere else, so a
//! failed match leaves the original, equivalent statements in place.

/// Whether `line` uses `name` as a whole identifier.
pub(crate) fn mentions(line: &str, name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(name).any(|(i, _)| {
        !line[..i].chars().next_back().is_some_and(word)
            && !line[i + name.len()..].chars().next().is_some_and(word)
    })
}
/// The line without navigation markers (`\0TOKEN\u{1}name\u{2}` around identifiers).
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find('\0') {
        out.push_str(&rest[..start]);
        rest = rest[start + 1..]
            .split_once('\u{1}')
            .map_or("", |(_, tail)| tail);
    }
    out.push_str(rest);
    out.replace('\u{2}', "")
}
pub(crate) fn depth(line: &str) -> usize {
    (line.len() - line.trim_start_matches(' ').len()) / 4
}
fn replace_word(line: &str, name: &str, with: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(name) {
        let whole = !rest[..i].chars().next_back().is_some_and(word)
            && !rest[i + name.len()..].chars().next().is_some_and(word);
        out.push_str(&rest[..i]);
        out.push_str(if whole { with } else { name });
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}
fn simple(expression: &str) -> bool {
    !expression.is_empty()
        && expression
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'))
}
// Writes that change `name` itself.
fn writes(line: &str, name: &str) -> bool {
    [
        "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
    ]
    .iter()
    .any(|op| line.contains(&format!("{name} {op} ")))
        || line.contains(&format!("{name}++"))
        || line.contains(&format!("{name}--"))
        || line.contains(&format!("ref {name}"))
        || line.contains(&format!("out {name}"))
}
/// The line index of the `}` closing the block opened by `{` at `open`.
fn close(lines: &[String], open: usize) -> Option<usize> {
    let d = depth(&lines[open]);
    (open + 1..lines.len()).find(|i| depth(&lines[*i]) == d && lines[*i].trim() == "}")
}
fn declared_names(lines: &[String]) -> Vec<String> {
    let mut names = vec![];
    for line in lines {
        let mut rest = line.as_str();
        while let Some(i) = rest.find("stack_") {
            let tail = &rest[i..];
            let end = tail
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(tail.len());
            let name = &tail[..end];
            if !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
            rest = &tail[end..];
        }
    }
    names
}
/// Removes evaluation-stack temporaries that only copy a variable: unread copies are
/// dropped, and a copy read only by the statements right after it is substituted.
pub fn stack_copies(lines: &mut Vec<String>) {
    for name in declared_names(lines) {
        let declaration = lines
            .iter()
            .position(|l| l.trim_end().ends_with(&format!(" {name};")) && !l.contains('='));
        let assigns: Vec<usize> = (0..lines.len())
            .filter(|i| {
                let t = lines[*i].trim();
                t.strip_prefix(&format!("{name} = "))
                    .and_then(|r| r.strip_suffix(';'))
                    .is_some_and(simple)
            })
            .collect();
        let others: Vec<usize> = (0..lines.len())
            .filter(|i| {
                Some(*i) != declaration && !assigns.contains(i) && mentions(&lines[*i], &name)
            })
            .collect();
        if others.is_empty() && !assigns.is_empty() {
            for i in assigns.iter().rev() {
                lines.remove(*i);
            }
        } else if let [assign] = assigns[..] {
            let source = lines[assign].trim()[name.len() + 3..]
                .trim_end_matches(';')
                .to_owned();
            let d = depth(&lines[assign]);
            let Some(&last) = others.last() else { continue };
            let contained = others.iter().all(|i| *i > assign)
                && (assign + 1..=last).all(|i| depth(&lines[i]) >= d || lines[i].trim().is_empty());
            let stable = (assign + 1..last).all(|i| {
                !writes(&lines[i], &source) && (!source.contains('.') || !lines[i].contains('('))
            });
            if !contained || !stable {
                continue;
            }
            for i in &others {
                lines[*i] = replace_word(&lines[*i], &name, &source);
            }
            lines.remove(assign);
        } else {
            continue;
        }
        if let Some(d) = lines
            .iter()
            .position(|l| l.trim_end().ends_with(&format!(" {name};")) && !l.contains('='))
            && !lines
                .iter()
                .enumerate()
                .any(|(i, l)| i != d && mentions(l, &name))
        {
            lines.remove(d);
        }
    }
}
/// `ref T v = ref x; v = E;` with no other use of `v` becomes `x = E;` (the IL shape
/// of `initobj` on a local).
pub fn ref_aliases(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 1 < lines.len() {
        let rewritten = (|| {
            let t = lines[i].trim().strip_prefix("ref ")?;
            let (_, name, target) = declaration(t)?;
            let target = target.strip_prefix("ref ")?;
            if !simple(target) {
                return None;
            }
            let value = lines[i + 1].trim().strip_prefix(&format!("{name} = "))?;
            if mentions(value, name)
                || lines
                    .iter()
                    .enumerate()
                    .any(|(j, l)| j != i && j != i + 1 && mentions(l, name))
            {
                return None;
            }
            Some(format!(
                "{}{target} = {value}",
                " ".repeat(depth(&lines[i]) * 4)
            ))
        })();
        if let Some(line) = rewritten {
            lines.splice(i..=i + 1, [line]);
        }
        i += 1;
    }
}
/// Removes compiler closure objects once lambdas refer to captured variables by name:
/// `C v = new C(); v.amount = amount;` disappears and `v.f` becomes `f`. A closure
/// that is used any other way (passed along, compared, …) is left unchanged.
pub fn closures(lines: &mut Vec<String>) {
    let mut i = 0;
    while i < lines.len() {
        let rewritten = (|| {
            let (ty, name, expression) = declaration(&lines[i])?;
            let ty = plain(ty);
            if !ty.contains("DisplayClass") || plain(expression) != format!("new {ty}()") {
                return None;
            }
            let prefix = format!("{name}.");
            let mut out = lines.clone();
            let mut declared = std::collections::HashSet::new();
            for (j, line) in lines.iter().enumerate() {
                if j == i || !mentions(line, name) {
                    continue;
                }
                let mut text = line.clone();
                // An initialization of a captured parameter copies it onto itself.
                let bare = plain(line);
                if let Some(rest) = bare.trim().strip_prefix(&prefix)
                    && let Some((field, value)) =
                        rest.strip_suffix(';').and_then(|r| r.split_once(" = "))
                    && field
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '<' || c == '>')
                {
                    if value == field {
                        out[j] = String::new();
                        continue;
                    }
                    if declared.insert(field.to_owned())
                        && !lines[..j]
                            .iter()
                            .any(|l| l.contains(&format!("{prefix}{field}")))
                    {
                        text = format!("{}var {field} = {value};", " ".repeat(depth(line) * 4));
                        out[j] = text;
                        continue;
                    }
                }
                text = replace_word(
                    &text.replace(&prefix, &format!("{name}\u{3}")),
                    name,
                    "\u{4}",
                )
                .replace("\u{4}\u{3}", "");
                if text.contains('\u{4}') || text.contains('\u{3}') {
                    return None;
                }
                out[j] = text;
            }
            // Cleared lines are removed below; blank source lines are kept.
            out[i] = String::new();
            Some(out)
        })();
        if let Some(next) = rewritten {
            let blank: Vec<usize> = (0..next.len())
                .filter(|j| next[*j].is_empty() && !lines[*j].is_empty())
                .collect();
            *lines = next;
            for j in blank.into_iter().rev() {
                lines.remove(j);
            }
            continue;
        }
        i += 1;
    }
}
/// Drops unused `T n = default(T);` / `T n;` lines from the method's leading
/// declaration block (from the opening brace to the first blank line).
pub fn unused_declarations(lines: &mut Vec<String>) {
    if lines.first().map(|l| l.trim()) != Some("{") {
        return;
    }
    let mut i = 1;
    while i < lines.len() && !lines[i].is_empty() && depth(&lines[i]) == 1 {
        let t = lines[i].trim().to_owned();
        let name = match declaration(&t) {
            Some((ty, name, value)) if value == format!("default({ty})") => Some(name.to_owned()),
            Some(_) => None,
            // Statement keywords are never declaration types.
            None if !t.contains(['=', '('])
                && t.ends_with(';')
                && ![
                    "throw", "return", "yield", "goto", "break", "continue", "await",
                ]
                .contains(&t.split(' ').next().unwrap_or("").trim_end_matches(';')) =>
            {
                t.trim_end_matches(';')
                    .rsplit_once(' ')
                    .map(|(_, n)| n.to_owned())
            }
            None => None,
        };
        // A statement ends the declaration block.
        let Some(name) = name else { break };
        if lines
            .iter()
            .enumerate()
            .any(|(j, l)| j != i && mentions(l, &name))
        {
            i += 1;
        } else {
            lines.remove(i);
        }
    }
    if i == 1 && lines.len() > 2 && lines[1].is_empty() {
        lines.remove(1);
    }
}
// Text that may be evaluated before a moved expression without reordering effects.
fn pure_prefix(text: &str) -> bool {
    !text.contains('(') && !text.contains('=') || text.trim_end().ends_with(" =")
}
/// `T v = E;` used once by the next statement (`return v;`, `x = … v …;`,
/// `yield return v;`) is folded into it when nothing with side effects is evaluated
/// before `v` there.
pub fn inline_temporaries(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 1 < lines.len() {
        let folded = (|| {
            let (_, name, value) = declaration(&lines[i])?;
            let next = &lines[i + 1];
            if depth(next) != depth(&lines[i]) || !next.trim_end().ends_with(';') {
                return None;
            }
            if lines
                .iter()
                .enumerate()
                .any(|(j, l)| j != i && j != i + 1 && mentions(l, name))
            {
                return None;
            }
            let uses: Vec<usize> = next
                .match_indices(name)
                .filter(|(p, _)| {
                    mentions(&next[*p..p + name.len()], name) && {
                        let word = |c: char| c.is_alphanumeric() || c == '_';
                        !next[..*p].chars().next_back().is_some_and(word)
                            && !next[p + name.len()..].chars().next().is_some_and(word)
                    }
                })
                .map(|(p, _)| p)
                .collect();
            let [at] = uses[..] else { return None };
            let before = &next[..at];
            let statement = before.trim_start();
            let allowed = statement.starts_with("return ")
                || statement.starts_with("yield return ")
                || statement.contains(" = ");
            if !allowed || !pure_prefix(before) {
                return None;
            }
            // Operators around the use keep precedence when the value is parenthesized.
            let simple = value
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
            let rest = &next[at + name.len()..];
            let tight = rest.trim_start().starts_with(';')
                && !before
                    .trim_end()
                    .ends_with(['+', '-', '*', '/', '%', '&', '|', '^', '<', '>', '!']);
            let value = if simple || tight {
                value.to_owned()
            } else {
                format!("({value})")
            };
            Some(format!("{before}{value}{rest}"))
        })();
        if let Some(line) = folded {
            lines.splice(i..=i + 1, [line]);
            continue;
        }
        i += 1;
    }
}
/// `x = x + 1;` → `x++;` and `x = x op E;` → `x op= E;` for plain identifiers.
pub fn compound_assignments(lines: &mut [String]) {
    for line in lines.iter_mut() {
        let indent = " ".repeat(depth(line) * 4);
        let Some((target, value)) = line
            .trim()
            .strip_suffix(';')
            .and_then(|t| t.split_once(" = "))
        else {
            continue;
        };
        if !target.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(rest) = value.strip_prefix(&format!("{target} ")) else {
            continue;
        };
        let Some((op, operand)) = rest.split_once(' ') else {
            continue;
        };
        if !["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"].contains(&op)
            || operand.contains(' ') && !operand.starts_with('(')
        {
            continue;
        }
        *line = if operand == "1" && (op == "+" || op == "-") {
            format!("{indent}{target}{op}{op};")
        } else {
            format!("{indent}{target} {op}= {operand};")
        };
    }
}
/// `T i = x; while (c) { …; i++; }` with `i` used only by the loop becomes
/// `for (T i = x; c; i++) { … }`. Loops containing `continue` keep their shape,
/// since `continue` would run the increment in a for statement.
pub fn while_to_for(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 3 < lines.len() {
        let rewritten = (|| {
            let (ty, name, init) = declaration(&lines[i])?;
            let condition = lines[i + 1]
                .trim()
                .strip_prefix("while (")?
                .strip_suffix(')')?;
            if !mentions(condition, name) || lines[i + 2].trim() != "{" {
                return None;
            }
            let end = close(lines, i + 2)?;
            let step = lines[end - 1].trim().strip_suffix(';')?;
            let increments = [format!("{name}++"), format!("{name}--")];
            if !increments.iter().any(|s| s == step)
                && !step.starts_with(&format!("{name} += "))
                && !step.starts_with(&format!("{name} -= "))
            {
                return None;
            }
            if depth(&lines[end - 1]) != depth(&lines[i]) + 1
                || lines[i + 3..end].iter().any(|l| l.trim() == "continue;")
                || lines[end + 1..].iter().any(|l| mentions(l, name))
            {
                return None;
            }
            let indent = " ".repeat(depth(&lines[i]) * 4);
            let mut replacement = vec![
                format!("{indent}for ({ty} {name} = {init}; {condition}; {step})"),
                format!("{indent}{{"),
            ];
            replacement.extend(lines[i + 3..end - 1].iter().cloned());
            replacement.push(format!("{indent}}}"));
            Some((end, replacement))
        })();
        if let Some((end, replacement)) = rewritten {
            lines.splice(i..=end, replacement);
        }
        i += 1;
    }
}
/// Removes `else { }` blocks left empty by earlier passes.
pub fn empty_else(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 2 < lines.len() {
        if lines[i].trim() == "else"
            && lines[i + 1].trim() == "{"
            && lines[i + 2].trim() == "}"
            && depth(&lines[i]) == depth(&lines[i + 2])
        {
            lines.drain(i..=i + 2);
            continue;
        }
        i += 1;
    }
}
/// `catch (T ex)` whose block never uses `ex` becomes `catch (T)`.
pub fn unused_catch_variables(lines: &mut [String]) {
    for i in 0..lines.len() {
        let t = lines[i].trim();
        let Some(inner) = t.strip_prefix("catch (").and_then(|r| r.strip_suffix(')')) else {
            continue;
        };
        let Some((ty, variable)) = inner.rsplit_once(' ') else {
            continue;
        };
        let Some(end) = lines
            .get(i + 1)
            .filter(|l| l.trim() == "{")
            .and_then(|_| close(lines, i + 1))
        else {
            continue;
        };
        let indent = " ".repeat(depth(&lines[i]) * 4);
        let used = (i + 2..end).any(|j| mentions(&lines[j], variable));
        // A general `catch` compiles to a System.Object clause; C# assemblies wrap
        // non-Exception throws, so a named variable is an Exception.
        let general = matches!(plain(ty).as_str(), "object" | "System.Object");
        lines[i] = match (general, used) {
            (true, false) => format!("{indent}catch"),
            (true, true) => format!(
                "{indent}catch ({} {variable})",
                if plain(ty).starts_with("System.") {
                    "System.Exception"
                } else {
                    "Exception"
                }
            ),
            (false, false) => format!("{indent}catch ({ty})"),
            (false, true) => continue,
        };
    }
}
// `T v = E;` split into (type, name, expression).
fn declaration(line: &str) -> Option<(&str, &str, &str)> {
    let (left, expression) = line.trim().strip_suffix(';')?.split_once(" = ")?;
    let (ty, name) = left.rsplit_once(' ')?;
    (!ty.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then_some((ty, name, expression))
}
/// `T v = E; if (v != null) { v.M(a); }` with no other use of `v` becomes `E?.M(a);`.
pub fn null_conditional(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 4 < lines.len() {
        let matched = (|| {
            let (_, name, expression) = declaration(&lines[i])?;
            let d = depth(&lines[i]);
            if lines[i + 1].trim() != format!("if ({name} != null)")
                || lines[i + 2].trim() != "{"
                || lines[i + 4].trim() != "}"
                || depth(&lines[i + 4]) != d
                || lines.get(i + 5).is_some_and(|l| l.trim() == "else")
            {
                return None;
            }
            let call = lines[i + 3].trim().strip_prefix(&format!("{name}."))?;
            if !call.ends_with(';') || mentions(call, name) {
                return None;
            }
            if lines
                .iter()
                .enumerate()
                .any(|(j, l)| !(i..=i + 4).contains(&j) && mentions(l, name))
            {
                return None;
            }
            let target = if expression.contains(' ') {
                format!("({expression})")
            } else {
                expression.to_owned()
            };
            Some(format!("{}{target}?.{call}", " ".repeat(d * 4)))
        })();
        if let Some(line) = matched {
            lines.splice(i..=i + 4, [line]);
        }
        i += 1;
    }
}
/// `T x = E; try { … } finally { if (x != null) { x.Dispose(); } }` becomes a `using`
/// statement when `x` is neither reassigned in the body nor used after it.
pub fn using_statements(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 2 < lines.len() {
        let rewritten = (|| {
            let (ty, name, expression) = declaration(&lines[i])?;
            let d = depth(&lines[i]);
            if lines[i + 1].trim() != "try" || lines[i + 2].trim() != "{" {
                return None;
            }
            let body_end = close(lines, i + 2)?;
            let finally = body_end + 1;
            if lines.get(finally)?.trim() != "finally" || lines.get(finally + 1)?.trim() != "{" {
                return None;
            }
            let end = close(lines, finally + 1)?;
            let handler: Vec<String> = lines[finally + 2..end]
                .iter()
                .map(|l| plain(l.trim()))
                .collect();
            let dispose = [
                format!("{name}.Dispose();"),
                format!("((IDisposable){name}).Dispose();"),
            ];
            let guarded = handler.len() == 4
                && handler[0] == format!("if ({name} != null)")
                && handler[1] == "{"
                && dispose.iter().any(|d| *d == handler[2])
                && handler[3] == "}";
            if !guarded
                || depth(&lines[body_end]) != d
                || (i + 3..body_end).any(|j| writes(&lines[j], name))
                || lines[end + 1..].iter().any(|l| mentions(l, name))
            {
                return None;
            }
            let indent = " ".repeat(d * 4);
            let mut replacement = vec![
                format!("{indent}using ({ty} {name} = {expression})"),
                format!("{indent}{{"),
            ];
            replacement.extend(lines[i + 3..body_end].iter().cloned());
            replacement.push(format!("{indent}}}"));
            Some((end, replacement))
        })();
        if let Some((end, replacement)) = rewritten {
            lines.splice(i..=end, replacement);
        }
        i += 1;
    }
}
/// `T[] a = E; for (int k = 0; k < a.Length; k++) { T x = a[k]; … }` becomes
/// `foreach (T x in E) { … }` when `a` and `k` serve only the loop.
pub fn foreach_arrays(lines: &mut Vec<String>) {
    let mut i = 0;
    while i + 3 < lines.len() {
        let rewritten = (|| {
            let (ty, array, source) = declaration(&lines[i])?;
            let element = ty.strip_suffix("[]")?;
            let d = depth(&lines[i]);
            let header = lines[i + 1].trim();
            let index = header.strip_prefix("for (int ")?.split(' ').next()?;
            if header != format!("for (int {index} = 0; {index} < {array}.Length; {index}++)")
                || lines[i + 2].trim() != "{"
            {
                return None;
            }
            let (item_ty, item, value) = declaration(&lines[i + 3])?;
            let end = close(lines, i + 2)?;
            if item_ty != element || value != format!("{array}[{index}]") || depth(&lines[end]) != d
            {
                return None;
            }
            let outside = lines
                .iter()
                .enumerate()
                .filter(|(j, _)| !(i..=i + 3).contains(j))
                .any(|(_, l)| mentions(l, array) || mentions(l, index));
            if outside || (i + 4..end).any(|j| writes(&lines[j], item)) {
                return None;
            }
            let indent = " ".repeat(d * 4);
            let mut replacement = vec![
                format!("{indent}foreach ({element} {item} in {source})"),
                format!("{indent}{{"),
            ];
            replacement.extend(lines[i + 4..end].iter().cloned());
            replacement.push(format!("{indent}}}"));
            Some((end, replacement))
        })();
        if let Some((end, replacement)) = rewritten {
            lines.splice(i..=end, replacement);
        }
        i += 1;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }
    #[test]
    fn copies_and_dead_stack_slots_disappear() {
        let mut body = lines(
            "{\n    EventHandler stack_001B_0;\n    EventHandler stack_001E_0;\n    if (v != null)\n    {\n        stack_001E_0 = v;\n        stack_001E_0.Invoke(this, e);\n    }\n    else\n    {\n        stack_001B_0 = v;\n    }\n}",
        );
        stack_copies(&mut body);
        assert_eq!(
            body,
            lines(
                "{\n    if (v != null)\n    {\n        v.Invoke(this, e);\n    }\n    else\n    {\n    }\n}"
            )
        );
        // A copy whose source changes before the use must stay.
        let mut kept = lines(
            "{\n    int stack_0001_0;\n    stack_0001_0 = n;\n    n = 5;\n    Use(stack_0001_0);\n}",
        );
        let original = kept.clone();
        stack_copies(&mut kept);
        assert_eq!(kept, original);
    }
    #[test]
    fn null_conditional_using_and_foreach() {
        let mut body = lines(
            "    EventHandler handler = this.Changed;\n    if (handler != null)\n    {\n        handler.Invoke(this, EventArgs.Empty);\n    }",
        );
        null_conditional(&mut body);
        assert_eq!(body, ["    this.Changed?.Invoke(this, EventArgs.Empty);"]);
        let mut body = lines(
            "    StringReader reader = new StringReader(path);\n    try\n    {\n        num = reader.Read();\n    }\n    finally\n    {\n        if (reader != null)\n        {\n            reader.Dispose();\n        }\n    }\n    return num;",
        );
        using_statements(&mut body);
        assert_eq!(
            body,
            lines(
                "    using (StringReader reader = new StringReader(path))\n    {\n        num = reader.Read();\n    }\n    return num;"
            )
        );
        let mut body = lines(
            "    int[] array = values;\n    for (int i = 0; i < array.Length; i++)\n    {\n        int item = array[i];\n        sum += item;\n    }",
        );
        foreach_arrays(&mut body);
        assert_eq!(
            body,
            lines("    foreach (int item in values)\n    {\n        sum += item;\n    }")
        );
        // The index is visible after the loop, so the loop stays a for statement.
        let mut used = lines(
            "    int[] array = values;\n    for (int i = 0; i < array.Length; i++)\n    {\n        int item = array[i];\n    }\n    return array;",
        );
        let original = used.clone();
        foreach_arrays(&mut used);
        assert_eq!(used, original);
    }
    #[test]
    fn closure_objects_become_captured_variables() {
        let mut body = lines(
            "    C.<>c__DisplayClass1_0 local = new C.<>c__DisplayClass1_0();\n    local.\u{0}04000001\u{1}amount\u{2} = amount;\n    local.total = 0;\n    Run(local.total);\n    return value => value + amount;",
        );
        closures(&mut body);
        assert_eq!(
            body,
            lines("    var total = 0;\n    Run(total);\n    return value => value + amount;")
        );
        // Passing the closure object itself keeps the original statements.
        let mut kept = lines(
            "    C.<>c__DisplayClass1_0 local = new C.<>c__DisplayClass1_0();\n    Use(local);",
        );
        let original = kept.clone();
        closures(&mut kept);
        assert_eq!(kept, original);
    }
    #[test]
    fn ref_alias_assignments_target_the_local() {
        let mut body =
            lines("    ref int? alias = ref value;\n    alias = default(int?);\n    return value;");
        ref_aliases(&mut body);
        assert_eq!(body, lines("    value = default(int?);\n    return value;"));
    }
    #[test]
    fn patterns_ignore_navigation_markers() {
        let mut body = lines(
            "    StringReader reader = new \u{0}02000001\u{1}StringReader\u{2}(path);\n    try\n    {\n        num = reader.\u{0}0A000001\u{1}Read\u{2}();\n    }\n    finally\n    {\n        if (reader != null)\n        {\n            reader.\u{0}0A000002\u{1}Dispose\u{2}();\n        }\n    }",
        );
        using_statements(&mut body);
        assert!(body[0].starts_with("    using (StringReader reader = new \u{0}02000001"));
        assert!(body[2].contains("\u{1}Read\u{2}"));
    }
    #[test]
    fn temporaries_fold_and_loops_become_for_statements() {
        let mut body = lines(
            "{\n    int num = default(int);\n    Exception stack_006F_0;\n\n    int i = 0;\n    while (i < n)\n    {\n        int num3 = i;\n        yield return num3;\n        int num2 = i;\n        int num4 = num2 + 1;\n        i = num4;\n    }\n    int total = value + 1;\n    return total;\n}",
        );
        unused_declarations(&mut body);
        inline_temporaries(&mut body);
        compound_assignments(&mut body);
        while_to_for(&mut body);
        assert_eq!(
            body,
            lines(
                "{\n    for (int i = 0; i < n; i++)\n    {\n        yield return i;\n    }\n    return value + 1;\n}"
            )
        );
        // Statements are never mistaken for declarations.
        let mut statements = lines("{\n    throw ex;\n}");
        unused_declarations(&mut statements);
        assert_eq!(statements, lines("{\n    throw ex;\n}"));
        // A call before the use would run before the moved call, so it stays.
        let mut kept = lines("    int v = First();\n    Use(Second(), v);");
        let original = kept.clone();
        inline_temporaries(&mut kept);
        assert_eq!(kept, original);
        // continue would skip the increment in a while loop but not in a for loop.
        let mut loop_with_continue = lines(
            "    int i = 0;\n    while (i < n)\n    {\n        continue;\n        i++;\n    }",
        );
        let original = loop_with_continue.clone();
        while_to_for(&mut loop_with_continue);
        assert_eq!(loop_with_continue, original);
    }
    #[test]
    fn unused_catch_variables_are_dropped() {
        let mut body = lines(
            "    catch (Exception ex)\n    {\n        num = 0;\n    }\n    catch (IOException ex2)\n    {\n        Log(ex2);\n    }",
        );
        unused_catch_variables(&mut body);
        assert_eq!(body[0], "    catch (Exception)");
        let mut general = lines("    catch (object ex)\n    {\n        num = 0;\n    }");
        unused_catch_variables(&mut general);
        assert_eq!(general[0], "    catch");
        assert_eq!(body[4], "    catch (IOException ex2)");
    }
}
