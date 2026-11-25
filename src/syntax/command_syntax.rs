//! MATLAB command syntax for any name: `format long g`, `warning off all`,
//! `myfn foo 'two words'`. At a statement start, a name that is not a
//! variable, followed by whitespace and then a word that does not read as an
//! operator expression, is a call with each word as a char argument.
//!
//! What the name follows decides (the documented rule): `=` (not `==`) is an
//! assignment and `(` a call; an operator with whitespace after it (`a - b`,
//! `a == b`) is an expression; anything else after the whitespace, including
//! an operator written against its operand (`a -b`, `ls ./d`), is a command.
//!
//! "Is a variable" is lexical: a name assigned earlier in the same code unit
//! (assignment target, function input or output, for-loop variable,
//! global/persistent, `catch err`), plus the names of the workspace the
//! host passes in for Command Window, -batch and eval text. A local function
//! in a script is its own unit; after its `end` the script's names (and the
//! workspace) apply again.
use super::{BLOCK_OPENERS, Result, Spanned, Token, keyword, line_break_len};
use std::collections::HashSet;

/// What a script's local function hides until its matching `end`.
struct Hidden<'a> {
    /// Open blocks before the `function` line.
    blocks: isize,
    variables: HashSet<String>,
    workspace: Option<&'a dyn Fn(&str) -> bool>,
}

/// Position inside a `classdef` file (outside local functions).
#[derive(Copy, Clone, PartialEq, Eq)]
enum ClassdefState {
    /// Not inside a `classdef ... end` block.
    None,
    /// Top level of a `classdef` block (header or between member blocks).
    Body,
    /// Inside `properties`, `events`, `enumeration`, or abstract `methods`.
    MemberDecl,
    /// Inside `methods` (where `function ... end` blocks live).
    Methods,
}

/// Lexical variable knowledge carried through one lexer pass.
pub(super) struct Scope<'a> {
    enabled: bool,
    workspace: Option<&'a dyn Fn(&str) -> bool>,
    variables: HashSet<String>,
    /// Token index where the current statement starts.
    statement: usize,
    /// The current statement's first `=` at bracket depth 0 was seen.
    assigned: bool,
    /// Inside `global`/`persistent` name lists.
    declaring: bool,
    /// Inside a `function` line, whose input names are variables.
    header: bool,
    /// Decided by the first statement: script code (Some(true)), or a
    /// function or class file (Some(false)), whose functions are separate
    /// units with nothing to restore.
    script: Option<bool>,
    /// Block openers minus `end`s at statement starts.
    blocks: isize,
    /// The enclosing units of the open local functions of a script.
    hidden: Vec<Hidden<'a>>,
    function_file: bool,
    possible_nested: bool,
    inherit_variables: bool,
    /// True right after a `function` header (and between consecutive `arguments` blocks).
    can_start_arguments: bool,
    /// True while lexing inside an `arguments ... end` block.
    in_arguments: bool,
    /// Current `classdef` block state, so `properties`/`events`/`enumeration`
    /// declarations do not trigger command syntax.
    classdef: ClassdefState,
    /// True while lexing the header line of a `methods (...)` block.
    methods_header: bool,
}

impl<'a> Scope<'a> {
    pub(super) fn new(enabled: bool, workspace: Option<&'a dyn Fn(&str) -> bool>) -> Self {
        Self {
            enabled,
            workspace,
            variables: HashSet::new(),
            statement: 0,
            assigned: false,
            declaring: false,
            header: false,
            script: None,
            blocks: 0,
            hidden: Vec::new(),
            function_file: false,
            possible_nested: false,
            inherit_variables: false,
            can_start_arguments: false,
            in_arguments: false,
            classdef: ClassdefState::None,
            methods_header: false,
        }
    }
    /// Implicit-end files have the same header prefix as nested definitions.
    /// Once the ends balance, lex an ambiguous file with lexical inheritance.
    pub(super) fn nested_pass(&self, failed: bool) -> Option<Self> {
        (self.possible_nested && (failed || self.blocks == 0)).then(|| Self {
            inherit_variables: true,
            ..Self::new(
                self.enabled,
                self.hidden.first().map_or(self.workspace, |h| h.workspace),
            )
        })
    }
    pub(super) fn ends_balanced(&self) -> bool {
        self.blocks == 0
    }
    pub(super) fn is_variable(&self, name: &str) -> bool {
        self.variables.contains(name)
            || self.workspace.is_some_and(|known| known(name))
            || self.inherit_variables
                && self
                    .hidden
                    .iter()
                    .any(|h| h.blocks > 0 && h.variables.contains(name))
    }
    /// True inside declaration blocks (`arguments`, `classdef` body,
    /// member declarations) where command syntax and file commands must not apply.
    pub(super) fn in_declaration_block(&self) -> bool {
        self.in_arguments
            || matches!(
                self.classdef,
                ClassdefState::Body | ClassdefState::MemberDecl
            )
    }
    /// The name just lexed at a statement start takes command syntax.
    pub(super) fn command(&self, source: &str, name: &str, after_name: usize) -> bool {
        self.enabled
            && !self.in_declaration_block()
            && !keyword(name)
            && !self.is_variable(name)
            && starts_command(source, after_name)
    }
    fn insert(&mut self, name: &str) {
        if !keyword(name) && !self.variables.contains(name) {
            self.variables.insert(name.to_owned());
        }
    }
    /// Track variables after the lexer pushed one ordinary token.
    /// `start`: the token begins a statement; `depth`: open brackets;
    /// `after`: the byte after the token.
    pub(super) fn observe(
        &mut self,
        tokens: &[Spanned],
        start: bool,
        depth: &[u8],
        source: &str,
        after: usize,
    ) {
        if !self.enabled {
            return;
        }
        let last = tokens.len() - 1;
        if start {
            self.statement = last;
            self.assigned = false;
            self.declaring = false;
        }
        if start {
            self.block(&tokens[last].token);
        }
        let previous = last.checked_sub(1).map(|i| &tokens[i].token);
        let before = last.checked_sub(2).map(|i| &tokens[i].token);
        match &tokens[last].token {
            Token::Newline | Token::Semi => {
                if self.header {
                    self.can_start_arguments = true;
                }
                if self.methods_header {
                    self.methods_header = false;
                    if has_abstract_attribute(&tokens[self.statement..last]) {
                        self.classdef = ClassdefState::MemberDecl;
                    }
                }
                self.header = false;
                self.declaring = false;
            }
            Token::Comma if depth.is_empty() => {
                if self.header {
                    self.can_start_arguments = true;
                }
                if self.methods_header {
                    self.methods_header = false;
                    if has_abstract_attribute(&tokens[self.statement..last]) {
                        self.classdef = ClassdefState::MemberDecl;
                    }
                }
                self.header = false;
                self.declaring = false;
            }
            Token::Name(name)
                if start && name == "function" && self.classdef != ClassdefState::MemberDecl =>
            {
                // Each function is its own code unit with its own workspace.
                self.variables.clear();
                self.workspace = None;
                self.header = true;
                self.can_start_arguments = false;
                self.in_arguments = false;
            }
            Token::Name(name) if start && self.in_arguments && name == "end" => {
                self.in_arguments = false;
            }
            Token::Name(name)
                if start
                    && !self.in_arguments
                    && self.can_start_arguments
                    && name == "arguments"
                    && starts_arguments_block(
                        source,
                        after,
                        self.variables.contains("arguments"),
                    ) =>
            {
                self.in_arguments = true;
            }
            Token::Name(name) if start && name == "end" => {
                if !self.in_arguments && !self.header {
                    self.can_start_arguments = false;
                }
                // The matching `end` of a script's local function.
                let blocks = self.blocks;
                if let Some(hidden) = self.hidden.pop_if(|hidden| hidden.blocks == blocks) {
                    self.variables = hidden.variables;
                    self.workspace = hidden.workspace;
                }
            }
            Token::Name(name) if start && matches!(name.as_str(), "global" | "persistent") => {
                if !self.in_arguments && !self.header {
                    self.can_start_arguments = false;
                }
                self.declaring = true;
            }
            Token::Name(name) if !keyword(name) => {
                if start && !self.in_arguments && !self.header {
                    self.can_start_arguments = false;
                }
                let input = self.header
                    && depth == b"("
                    && matches!(previous, Some(Token::LParen | Token::Comma));
                let looped = named(previous, &["for", "parfor"])
                    || matches!(previous, Some(Token::LParen)) && named(before, &["for", "parfor"]);
                let caught = named(previous, &["catch"]) && line_ends(source, after);
                if self.declaring || input || looped || caught {
                    self.insert(name);
                }
            }
            Token::Name(_) if start && !self.in_arguments && !self.header => {
                self.can_start_arguments = false;
            }
            Token::Eq if depth.is_empty() && !self.assigned => {
                self.assigned = true;
                self.assignment(tokens);
            }
            _ => {}
        }
    }
    /// Count blocks at a statement start; a `function` line in a script
    /// keeps the enclosing unit's names and workspace aside until its `end`.
    fn block(&mut self, token: &Token) {
        if matches!(token, Token::Newline | Token::Semi | Token::Comma) {
            return;
        }
        let name = match token {
            Token::Name(name) => name.as_str(),
            _ => "",
        };
        let first_statement = self.script.is_none();
        let script = *self
            .script
            .get_or_insert(!matches!(name, "function" | "classdef"));
        if first_statement {
            self.function_file = name == "function";
        }
        let child = name == "function"
            && self.blocks > 0
            && (self.function_file || script && !self.hidden.is_empty());
        self.possible_nested |= child;
        if name == "function" && script || child && self.inherit_variables {
            self.hidden.push(Hidden {
                blocks: self.blocks,
                variables: std::mem::take(&mut self.variables),
                workspace: self.workspace,
            });
        }
        match self.classdef {
            ClassdefState::None if first_statement && name == "classdef" => {
                self.classdef = ClassdefState::Body;
            }
            ClassdefState::Body => match name {
                "properties" | "events" | "enumeration" => {
                    self.classdef = ClassdefState::MemberDecl;
                }
                "methods" => {
                    self.classdef = ClassdefState::Methods;
                    self.methods_header = true;
                }
                "end" => {
                    self.classdef = ClassdefState::None;
                }
                _ => {}
            },
            ClassdefState::MemberDecl => {
                if name == "end" {
                    self.classdef = ClassdefState::Body;
                }
            }
            ClassdefState::Methods if name == "end" && self.blocks == 0 && !self.in_arguments => {
                self.classdef = ClassdefState::Body;
            }
            _ => {
                if BLOCK_OPENERS.contains(&name) {
                    self.blocks += 1;
                } else if name == "end" && !self.in_arguments {
                    self.blocks -= 1;
                }
            }
        }
    }
    /// The root names left of the statement's `=`: `x`, `x(2)`, `s.a{1}`,
    /// `[a, b(3), ~]`, and a function's outputs.
    fn assignment(&mut self, tokens: &[Spanned]) {
        let mut first = self.statement;
        if matches!(&tokens[first].token, Token::Name(n) if n == "function") {
            first += 1;
        }
        match tokens.get(first).map(|t| &t.token) {
            Some(Token::Name(name)) => self.insert(name),
            Some(Token::LBracket) => {
                let mut depth = 0usize;
                for (offset, spanned) in tokens[first..].iter().enumerate() {
                    match &spanned.token {
                        Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
                        Token::RParen | Token::RBracket | Token::RBrace => {
                            depth = depth.saturating_sub(1);
                        }
                        Token::Name(name)
                            if depth == 1 && tokens[first + offset - 1].token != Token::Dot =>
                        {
                            self.insert(name);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn has_abstract_attribute(tokens: &[Spanned]) -> bool {
    let mut depth = 0usize;
    let mut is_abstract = false;
    for (i, spanned) in tokens.iter().enumerate() {
        match &spanned.token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Name(name) if depth == 1 && name == "Abstract" => {
                let previous = tokens[..i].last().map(|t| &t.token);
                let next = tokens.get(i + 1).map(|t| &t.token);
                is_abstract = match (previous, next) {
                    (Some(Token::Ignore), _) => false,
                    (Some(Token::LParen | Token::Comma), Some(Token::Comma | Token::RParen)) => {
                        true
                    }
                    (Some(Token::LParen | Token::Comma), Some(Token::Eq)) => matches!(
                        tokens.get(i + 2).map(|t| &t.token),
                        Some(Token::Name(v) | Token::String(v) | Token::DoubleQuoted(v)) if v == "true"
                    ),
                    _ => continue,
                };
            }
            _ => {}
        }
    }
    is_abstract
}

fn named(token: Option<&Token>, words: &[&str]) -> bool {
    matches!(token, Some(Token::Name(n)) if words.contains(&n.as_str()))
}

/// True if `arguments` at a function body start begins an `arguments` block
/// (`arguments`, `arguments (Input)`, `arguments (Output)`, etc.) rather than
/// an assignment or expression like `arguments = 1`.
fn starts_arguments_block(source: &str, mut at: usize, is_var: bool) -> bool {
    let bytes = source.as_bytes();
    while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        at += 1;
    }
    let has_attr = bytes.get(at) == Some(&b'(');
    if is_var && !has_attr {
        return false;
    }
    if has_attr {
        at += 1;
        while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
            at += 1;
        }
        let start = at;
        while bytes
            .get(at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            at += 1;
        }
        let attr = &source[start..at];
        if !matches!(attr, "Input" | "Output" | "Repeating") {
            return false;
        }
        while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
            at += 1;
        }
        if bytes.get(at) != Some(&b')') {
            return false;
        }
        at += 1;
        while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
            at += 1;
        }
    }
    bytes.get(at).is_none_or(|c| {
        matches!(c, b'\r' | b'\n' | b';' | b',' | b'%') || bytes[at..].starts_with(b"...")
    })
}

/// Only blanks or a comment follow on this line (`catch err`).
fn line_ends(source: &str, mut at: usize) -> bool {
    let bytes = source.as_bytes();
    while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        at += 1;
    }
    bytes
        .get(at)
        .is_none_or(|c| matches!(c, b'\r' | b'\n' | b';' | b',' | b'%'))
}

/// Operators that make `name OP rest` an expression when whitespace follows
/// them. Longest first.
const OPERATORS: [&[u8]; 22] = [
    b"==", b"~=", b"<=", b">=", b"&&", b"||", b".*", b"./", b".\\", b".^", b".'", b"+", b"-", b"*",
    b"/", b"\\", b"^", b"<", b">", b"&", b"|", b":",
];

/// The documented rule for the text after a name: whitespace, then anything
/// but `=`, `(`, a statement end, a comment, a continuation or an operator
/// followed by whitespace. Also not a command: a brace group then `=`
/// (`c {2} = 5` assigns, as before command syntax for any name).
pub(super) fn starts_command(source: &str, after_name: usize) -> bool {
    let bytes = source.as_bytes();
    if !bytes
        .get(after_name)
        .is_some_and(|c| matches!(c, b' ' | b'\t'))
    {
        return false;
    }
    let mut at = after_name;
    while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        at += 1;
    }
    let rest = &bytes[at..];
    let Some(&first) = rest.first() else {
        return false;
    };
    if b"\r\n;,%(".contains(&first) || rest.starts_with(b"...") {
        return false;
    }
    if first == b'=' && rest.get(1) != Some(&b'=') {
        return false;
    }
    if first == b'{' && brace_assignment(rest) {
        return false;
    }
    if let Some(operator) = OPERATORS.iter().find(|op| rest.starts_with(op)) {
        return rest
            .get(operator.len())
            .is_some_and(|c| !c.is_ascii_whitespace());
    }
    true
}

/// `rest` starts with a balanced `{...}` on this line followed by blanks and
/// a single `=`. Braces are counted as written (quotes are not read).
fn brace_assignment(rest: &[u8]) -> bool {
    let mut depth = 0usize;
    for (at, &c) in rest.iter().enumerate() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let after = rest[at + 1..]
                        .iter()
                        .position(|c| !matches!(c, b' ' | b'\t'))
                        .map_or(&[][..], |blank| &rest[at + 1 + blank..]);
                    return after.first() == Some(&b'=') && after.get(1) != Some(&b'=');
                }
            }
            b'\r' | b'\n' => return false,
            _ => {}
        }
    }
    false
}

/// Lex the words after a command name as `( 'w1' , 'w2' )`. Words split on
/// blanks; a single-quoted part keeps its blanks and loses its quotes (`''`
/// is one quote); double quotes are ordinary characters. `;`, `,`, a line
/// break or the end end the command; `%` starts a comment and `...` a
/// continuation. `raw` (disp): several words stay one argument, the source
/// text as written, and an unmatched quote is an ordinary character.
pub(super) fn lex(
    source: &str,
    at: &mut usize,
    tokens: &mut Vec<Spanned>,
    error: &mut usize,
    raw: bool,
) -> Result<()> {
    let bytes = source.as_bytes();
    let open = *at;
    let mut words: Vec<(String, usize, usize)> = Vec::new();
    loop {
        while bytes.get(*at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
            *at += 1;
        }
        if bytes[*at..].starts_with(b"...") {
            while *at < bytes.len() && line_break_len(bytes, *at) == 0 {
                *at += 1;
            }
            *at += line_break_len(bytes, *at);
            continue;
        }
        if *at == bytes.len() || b"\r\n;,%".contains(&bytes[*at]) {
            break;
        }
        let start = *at;
        *error = start;
        let mut text = String::new();
        while *at < bytes.len()
            && !b" \t\r\n;,%".contains(&bytes[*at])
            && !bytes[*at..].starts_with(b"...")
        {
            if bytes[*at] == b'\'' {
                match quoted(source, *at + 1) {
                    Some((inner, end)) => {
                        text.push_str(&inner);
                        *at = end;
                    }
                    None if raw => {
                        text.push('\'');
                        *at += 1;
                    }
                    None => return Err("A quoted command argument is not terminated".into()),
                }
                continue;
            }
            let ch = source[*at..].chars().next().ok_or("Invalid command text")?;
            text.push(ch);
            *at += ch.len_utf8();
        }
        words.push((text, start, *at));
        if words.len() > 255 {
            return Err("A command takes at most 255 words".into());
        }
    }
    if raw && words.len() > 1 {
        let (start, end) = (words[0].1, words[words.len() - 1].2);
        words = vec![(source[start..end].to_owned(), start, end)];
    }
    tokens.push(Spanned {
        token: Token::LParen,
        start: open,
        end: open,
    });
    for (index, (text, start, end)) in words.into_iter().enumerate() {
        if index > 0 {
            tokens.push(Spanned {
                token: Token::Comma,
                start,
                end: start,
            });
        }
        tokens.push(Spanned {
            token: Token::String(text),
            start,
            end,
        });
    }
    let end = tokens.last().map_or(*at, |t| t.end);
    tokens.push(Spanned {
        token: Token::RParen,
        start: end,
        end,
    });
    if tokens.len() > 100_000 {
        return Err("Script exceeds 100000 tokens".into());
    }
    Ok(())
}

/// The text of a quoted part starting after its `'`, and the byte after the
/// closing quote; None when the line ends first.
fn quoted(source: &str, mut at: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    let mut text = String::new();
    while at < bytes.len() && line_break_len(bytes, at) == 0 {
        if bytes[at] == b'\'' {
            if bytes.get(at + 1) == Some(&b'\'') {
                text.push('\'');
                at += 2;
                continue;
            }
            return Some((text, at + 1));
        }
        let ch = source[at..].chars().next()?;
        text.push(ch);
        at += ch.len_utf8();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::starts_command;

    fn command(line: &str) -> bool {
        let name = line.find([' ', '\t']).unwrap_or(line.len());
        starts_command(line, name)
    }

    #[test]
    fn documented_rule() {
        for line in [
            "format long g",
            "ls ./d",
            "a -b",
            "myfn -1",
            "a ==b",
            "disp x+1",
            "myfn 'two words'",
            "disp \"s\"",
            "a\tb",
            "a [1 2]",
            "a ~b",
            "c {2}",
            "disp {x}",
            "c {2} == 5",
            "c {2 = 5",
            "c {1}{2} = 5",
        ] {
            assert!(command(line), "{line}");
        }
        for line in [
            "a = b",
            "a =b",
            "a(1)",
            "a (1)",
            "a - b",
            "a == b",
            "ls ./ d",
            "a",
            "a ",
            "a;",
            "a ,b",
            "a % note",
            "a ...",
            "a'",
            "a -",
            "a .^ 2",
            "a : b",
            "a.b",
            "c {2} = 5",
            "c {1, {2}} =5",
            "c\t{2}\t= 'x'",
        ] {
            assert!(!command(line), "{line}");
        }
    }

    /// `x` before `-1` lexes as a variable (`x - 1`), not a command.
    fn subtracts(source: &str, workspace: Option<&dyn Fn(&str) -> bool>) -> bool {
        use super::super::{Token, lex_in};
        let mut byte = 0;
        let tokens = lex_in(source, &mut byte, super::Scope::new(true, workspace)).unwrap();
        let at = tokens
            .iter()
            .rposition(|t| t.token == Token::Name("x".into()))
            .unwrap();
        tokens[at + 1].token == Token::Minus
    }

    #[test]
    fn script_names_come_back_after_a_local_function() {
        let body = "function y = f(t)\n  if t, y = t; end\n  x -1\nend\n";
        // Script code first: the function hides `x`, its `end` restores it.
        assert!(subtracts(&format!("x = 1;\n{body}x -1\n"), None));
        assert!(!subtracts(&format!("x = 1;\n{body}"), None));
        // A nested function inside a local function: both ends restore.
        let nested = "x = 1;\nfunction f\n function g\n end\nend\nx -1\n";
        assert!(subtracts(nested, None));
        assert!(!subtracts(
            "x = 1;\nfunction f\n function g\n end\n x -1\nend\n",
            None
        ));
        // The workspace predicate comes back too.
        let known: &dyn Fn(&str) -> bool = &|n| n == "x";
        assert!(subtracts(&format!("disp(1)\n{body}x -1\n"), Some(known)));
        // A function file: each function is its own unit, nothing restored.
        assert!(!subtracts(
            "function a\n x = 1;\nend\nfunction b\n x -1\nend\n",
            None
        ));
    }
}
