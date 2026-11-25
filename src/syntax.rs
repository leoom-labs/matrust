use crate::Result;
mod class_decl;
mod command_syntax;
mod file_commands;
mod function_decl;
pub use class_decl::{
    Access, ClassDefinition, ClassFileError, EnumMember, MethodAttributes, MethodBlock, Property,
    PropertyAttributes, PropertyBlock, is_class_file, parse_class_file,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Number(f64),
    Imaginary(f64),
    Name(String),
    String(String),
    DoubleQuoted(String),
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Power,
    DotStar,
    DotSlash,
    DotBackslash,
    DotPower,
    Transpose,
    NonConjugateTranspose,
    Eq,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    ShortAnd,
    ShortOr,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Newline,
    Colon,
    Ignore,
    /// Field access dot, only before a field name or `(`.
    Dot,
    Eof,
    At,
}
#[derive(Clone, Debug)]
struct Spanned {
    token: Token,
    start: usize,
    end: usize,
}
#[derive(Clone, Debug)]
pub enum Expr {
    Number(f64),
    Imaginary(f64),
    Name(String),
    /// A `Name=Value` argument. It becomes the text name only when a call is
    /// evaluated; indexing must reject it rather than treating it as text.
    NameValue(String),
    String(String),
    DoubleQuoted(String),
    All,
    Unary(bool, Box<Expr>),
    Not(Box<Expr>),
    Binary(Token, Box<Expr>, Box<Expr>),
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Transpose(Box<Expr>),
    NonConjugateTranspose(Box<Expr>),
    Call(String, Vec<Expr>),
    Matrix(Vec<Vec<Expr>>),
    CellLiteral(Vec<Vec<Expr>>),
    CellContents(String, Vec<Expr>),
    /// `base.name` for any non-name-rooted-index base.
    Field(Box<Expr>, String),
    /// `base.(expression)`.
    DynamicField(Box<Expr>, Box<Expr>),
    /// `base(args)` on a value that is not a bare variable name (`s.a(2)`).
    Index(Box<Expr>, Vec<Expr>),
    /// `base{args}` on a value that is not a bare variable name (`s.c{2}`).
    Brace(Box<Expr>, Vec<Expr>),
    /// `@name`: resolved when the handle is called.
    FunctionHandle(String),
    /// `@(params) body`: captures referenced workspace variables when evaluated.
    Anonymous(std::sync::Arc<AnonymousFunction>),
    /// `f(1)(2)`: arguments applied to the value of a call, as for handles
    /// that return handles. Needs no space before the second `(`.
    Apply(Box<Expr>, Vec<Expr>),
}
impl Expr {
    /// Forms that may yield a comma-separated list of values.
    pub(crate) fn may_expand(&self) -> bool {
        matches!(
            self,
            Self::CellContents(..) | Self::Field(..) | Self::DynamicField(..) | Self::Brace(..)
        )
    }
}
/// One accessor of an assignment target such as `s(2).a.b{3}`.
#[derive(Clone, Debug)]
pub enum PathStep {
    Paren(Vec<Expr>),
    Brace(Vec<Expr>),
    Field(String),
    Dynamic(Expr),
}
/// Parsed anonymous function. `text` is the MATLAB `func2str` form, e.g.
/// `@(x)x.^2+1`: the body tokens re-joined without spaces.
#[derive(Debug)]
pub struct AnonymousFunction {
    pub params: Vec<String>,
    pub body: Expr,
    pub text: String,
}
#[derive(Clone, Debug)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: std::ops::Range<usize>,
}
#[derive(Clone, Debug)]
pub enum StatementKind {
    Declaration {
        persistent: bool,
        names: Vec<String>,
    },
    Function(Box<FunctionDefinition>),
    Evaluate(Evaluation),
    If {
        branches: Vec<ConditionalBranch>,
        otherwise: Vec<Statement>,
    },
    While {
        condition: Expr,
        body: Vec<Statement>,
    },
    For {
        name: String,
        values: Expr,
        body: Vec<Statement>,
    },
    /// Serial execution of parfor/spmd after evaluating their worker arguments.
    SerialBlock {
        arguments: Vec<Expr>,
        body: Vec<Statement>,
    },
    Try {
        body: Vec<Statement>,
        /// `catch err` identifier, if any.
        error: Option<String>,
        handler: Vec<Statement>,
    },
    Break,
    Continue,
    Return,
    Switch {
        subject: Expr,
        cases: Vec<SwitchCase>,
        otherwise: Vec<Statement>,
    },
}
#[derive(Clone, Debug)]
pub struct SwitchCase {
    pub value: Expr,
    pub body: Vec<Statement>,
    pub byte: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgumentsKind {
    Input,
    Output,
    Repeating,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgDim {
    Fixed(usize),
    Colon,
}
#[derive(Clone, Debug)]
pub struct ArgValidator {
    pub name: String,
    pub args: Vec<Expr>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct ArgumentSpec {
    /// For name-value arguments (`opts.Name`), `group` is `Some("opts")` and `name` is `"Name"`.
    pub group: Option<String>,
    pub name: String,
    pub dims: Option<Vec<ArgDim>>,
    pub class_name: Option<String>,
    pub validators: Vec<ArgValidator>,
    pub default: Option<Expr>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct ArgumentsBlock {
    pub kind: ArgumentsKind,
    pub specs: Vec<ArgumentSpec>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct FunctionDefinition {
    pub name: String,
    pub inputs: Vec<Option<String>>,
    pub outputs: Vec<String>,
    pub arguments: Vec<ArgumentsBlock>,
    pub body: Vec<Statement>,
    pub explicit_end: bool,
}
#[derive(Clone, Debug)]
pub struct ConditionalBranch {
    pub condition: Expr,
    pub body: Vec<Statement>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct Evaluation {
    /// Ordinary named outputs; None discards an output (~).
    pub targets: Vec<Option<String>>,
    /// Indexed destination, mutually exclusive with named outputs.
    pub indexed_target: Option<(String, Vec<Expr>)>,
    pub cell_content_target: bool,
    /// Assignment through fields, e.g. `s(2).a.b = v`: root name and accessors.
    pub path_target: Option<(String, Vec<PathStep>)>,
    /// Bracketed destinations, including comma-separated fields/cell contents.
    pub list_target: Option<Vec<Option<Expr>>>,
    pub expr: Expr,
    pub quiet: bool,
}

pub(crate) fn keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "elseif"
            | "else"
            | "end"
            | "for"
            | "while"
            | "break"
            | "continue"
            | "return"
            | "function"
            | "switch"
            | "case"
            | "otherwise"
            | "try"
            | "catch"
            | "global"
            | "persistent"
            | "parfor"
            | "spmd"
            | "classdef"
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub byte: usize,
    pub line: usize,
    pub column: usize,
}
/// Number of source bytes in one physical line ending. CRLF is one break.
/// This does not rewrite the source or interpret characters inside literals.
#[doc(hidden)]
pub fn line_break_len(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at) {
        Some(b'\r') => {
            if bytes.get(at + 1) == Some(&b'\n') {
                2
            } else {
                1
            }
        }
        Some(b'\n') => 1,
        _ => 0,
    }
}
impl Diagnostic {
    pub fn new(source: &str, mut byte: usize, message: String) -> Self {
        byte = byte.min(source.len());
        while !source.is_char_boundary(byte) {
            byte -= 1;
        }
        let prefix = &source[..byte];
        let (mut line, mut start, mut at) = (1, 0, 0);
        while at < prefix.len() {
            let width = line_break_len(source.as_bytes(), at);
            if width != 0 {
                line += 1;
                at = (at + width).min(prefix.len());
                start = at;
            } else {
                at += 1;
            }
        }
        let column = source[start..byte].chars().count() + 1;
        Self {
            message,
            byte,
            line,
            column,
        }
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

/// The byte where the line holding `at` breaks (or the end of input).
fn line_end(b: &[u8], mut at: usize) -> usize {
    while at < b.len() && line_break_len(b, at) == 0 {
        at += 1;
    }
    at
}
/// The line around `at` holds only blanks besides `marker` (`%{` or `%}`),
/// which starts at `at`.
fn lone_marker(b: &[u8], at: usize, marker: &[u8]) -> bool {
    let blank = |c: &u8| matches!(c, b' ' | b'\t');
    let line_start = b[..at]
        .iter()
        .rposition(|c| !blank(c))
        .is_none_or(|before| matches!(b[before], b'\n' | b'\r'));
    let end = line_end(b, at);
    line_start && b[at..end].starts_with(marker) && b[at + marker.len()..end].iter().all(blank)
}
/// A `%{` alone on its line (blanks allowed) opens a block comment; with any
/// other text on the line it is an ordinary `%` comment.
fn block_comment_opens(b: &[u8], at: usize) -> bool {
    lone_marker(b, at, b"%{")
}
/// For a block comment opening at `at`: the byte after the matching `%}`
/// (before its line break) and whether it closed. Block comments nest: each
/// inner lone `%{` line needs its own lone `%}` line. Unclosed, the comment
/// runs to the end of input.
fn block_comment_end(b: &[u8], at: usize) -> (usize, bool) {
    let mut depth = 1usize;
    let mut i = line_end(b, at);
    while i < b.len() {
        i += line_break_len(b, i);
        let start = i + b[i..]
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count();
        i = line_end(b, i);
        if lone_marker(b, start, b"%{") {
            depth += 1;
        } else if lone_marker(b, start, b"%}") {
            depth -= 1;
            if depth == 0 {
                return (i, true);
            }
        }
    }
    (b.len(), false)
}

fn lex(source: &str, error_byte: &mut usize) -> Result<Vec<Spanned>> {
    lex_in(source, error_byte, command_syntax::Scope::new(true, None))
}
/// Lex with command syntax decided by `scope` (see `command_syntax`).
fn lex_in(
    source: &str,
    error_byte: &mut usize,
    mut scope: command_syntax::Scope,
) -> Result<Vec<Spanned>> {
    let first = lex_scoped_tokens(source, error_byte, &mut scope);
    if let Some(mut nested) = scope.nested_pass(first.is_err()) {
        let first_byte = *error_byte;
        let result = lex_scoped_tokens(source, error_byte, &mut nested);
        if first.is_ok() || nested.ends_balanced() {
            return result;
        }
        *error_byte = first_byte;
    }
    first
}

fn lex_scoped_tokens(
    source: &str,
    error_byte: &mut usize,
    scope: &mut command_syntax::Scope,
) -> Result<Vec<Spanned>> {
    let b = source.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();
    let mut delimiters = Vec::new();
    let mut anonymous_params = Vec::new();
    let mut after_anonymous_params = false;
    let mut condition_header = false;
    while i < b.len() {
        let start = i;
        *error_byte = start;
        if b[i] == b' ' || b[i] == b'\t' {
            i += 1;
            continue;
        }
        if b[i] == b'%' {
            i = if block_comment_opens(b, i) {
                block_comment_end(b, i).0
            } else {
                line_end(b, i)
            };
            continue;
        }
        if b[i..].starts_with(b"...") {
            while i < b.len() && line_break_len(b, i) == 0 {
                i += 1;
            }
            i += line_break_len(b, i);
            continue;
        }
        let token = if b[i].is_ascii_digit()
            || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if b.get(i) == Some(&b'.') && !b.get(i + 1).is_some_and(|v| b"*/\\^'".contains(v)) {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if b.get(i).is_some_and(|v| *v == b'e' || *v == b'E') {
                i += 1;
                if b.get(i).is_some_and(|v| *v == b'+' || *v == b'-') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let number = source[start..i].parse().map_err(|_| "Invalid number")?;
            if matches!(b.get(i), Some(b'i' | b'j')) {
                i += 1;
                if b.get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    return Err("Invalid imaginary numeric suffix".into());
                }
                Token::Imaginary(number)
            } else {
                Token::Number(number)
            }
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Token::Name(source[start..i].into())
        } else {
            let previous_can_end = !after_anonymous_params
                && tokens.last().is_some_and(|t: &Spanned| {
                    matches!(
                        &t.token,
                        Token::Number(_)
                            | Token::Imaginary(_)
                            | Token::RParen
                            | Token::RBracket
                            | Token::RBrace
                            | Token::Transpose
                            | Token::NonConjugateTranspose
                    ) || matches!(&t.token, Token::Name(name) if name == "end" || !keyword(name))
                });
            let ch = b[i];
            i += 1;
            match ch {
                b'\''
                    if previous_can_end
                        && !(matches!(delimiters.last(), Some(b'[' | b'{'))
                            && tokens.last().is_some_and(|t| t.end < start)) =>
                {
                    Token::Transpose
                }
                b'\'' | b'"' => {
                    let quote = ch;
                    let mut value = String::new();
                    let mut closed = false;
                    while i < b.len() {
                        if b[i] == quote {
                            i += 1;
                            if b.get(i) == Some(&quote) {
                                value.push(quote as char);
                                i += 1;
                            } else {
                                closed = true;
                                break;
                            }
                        } else {
                            let c = source[i..].chars().next().ok_or("Invalid string")?;
                            value.push(c);
                            i += c.len_utf8();
                        }
                    }
                    if !closed {
                        return Err("Unterminated string".into());
                    }
                    if quote == b'"' {
                        Token::DoubleQuoted(value)
                    } else {
                        Token::String(value)
                    }
                }
                b'+' => Token::Plus,
                b'-' => Token::Minus,
                b'*' => Token::Star,
                b'/' => Token::Slash,
                b'\\' => Token::Backslash,
                b'^' => Token::Power,
                b'=' if b.get(i) == Some(&b'=') => {
                    i += 1;
                    Token::Equal
                }
                b'=' => Token::Eq,
                b'<' if b.get(i) == Some(&b'=') => {
                    i += 1;
                    Token::LessEqual
                }
                b'<' => Token::Less,
                b'>' if b.get(i) == Some(&b'=') => {
                    i += 1;
                    Token::GreaterEqual
                }
                b'>' => Token::Greater,
                b'&' if b.get(i) == Some(&b'&') => {
                    i += 1;
                    Token::ShortAnd
                }
                b'&' => Token::And,
                b'|' if b.get(i) == Some(&b'|') => {
                    i += 1;
                    Token::ShortOr
                }
                b'|' => Token::Or,
                b'(' => Token::LParen,
                b')' => Token::RParen,
                b'[' => Token::LBracket,
                b']' => Token::RBracket,
                b'{' => Token::LBrace,
                b'}' => Token::RBrace,
                b',' => Token::Comma,
                b';' => Token::Semi,
                b'\r' => {
                    if b.get(i) == Some(&b'\n') {
                        i += 1;
                    }
                    Token::Newline
                }
                b'\n' => Token::Newline,
                b':' => Token::Colon,
                b'~' if b.get(i) == Some(&b'=') => {
                    i += 1;
                    Token::NotEqual
                }
                b'~' => Token::Ignore,
                b'@' => Token::At,
                b'.' => {
                    let next = *b.get(i).ok_or("Unexpected dot")?;
                    if next.is_ascii_alphabetic() || next == b'_' || next == b'(' {
                        Token::Dot
                    } else {
                        i += 1;
                        match next {
                            b'*' => Token::DotStar,
                            b'/' => Token::DotSlash,
                            b'\\' => Token::DotBackslash,
                            b'^' => Token::DotPower,
                            b'\'' => Token::NonConjugateTranspose,
                            _ => {
                                return Err("Unsupported dotted operator".into());
                            }
                        }
                    }
                }
                _ => return Err("Unsupported character".into()),
            }
        };
        let after_separator = tokens.last().is_none_or(|t| is_separator(&t.token));
        // The lexer only needs the unambiguous starts that affect command syntax
        // or assignment targets. The parser consumes the full condition first.
        let after_condition = condition_header
            && delimiters.is_empty()
            && !after_anonymous_params
            && tokens.last().is_some_and(|t| t.token == Token::RParen)
            && (matches!(&token, Token::Name(name) if !ends_block(name))
                || token == Token::LBracket);
        let file_command = !scope.in_declaration_block()
            && delimiters.is_empty()
            && (after_separator || after_condition)
            && matches!(&token, Token::Name(name) if file_commands::is_command_name(name)
                && !scope.is_variable(name)
                && (name != "print" || file_commands::print_arguments(source, i))
                && (file_commands::arguments(name, source, i)
                    || matches!(name.as_str(), "dir" | "ls" | "delete" | "copyfile" | "movefile")
                        && file_commands::envx_wildcard_argument(source, i)));
        // A statement can also follow `try`, `catch`, `else` or `otherwise`.
        let statement_start = delimiters.is_empty()
            && (after_separator
                || after_condition
                || tokens.last().is_some_and(|t| matches!(&t.token, Token::Name(n) if matches!(n.as_str(), "try" | "catch" | "else" | "otherwise"))));
        if statement_start || delimiters.is_empty() && is_separator(&token) {
            condition_header = statement_start
                && matches!(&token, Token::Name(name) if matches!(name.as_str(), "if" | "elseif" | "while"));
        }
        // Any other name: MATLAB command syntax (`format long g`, `myfn a b`).
        let command = !file_command
            && statement_start
            && matches!(&token, Token::Name(name) if !file_commands::is_command_name(name) && scope.command(source, name, i));
        let raw_command = command && matches!(&token, Token::Name(name) if name == "disp");
        after_anonymous_params = false;
        match &token {
            Token::LParen => {
                anonymous_params.push((
                    delimiters.len(),
                    tokens.last().is_some_and(|t| t.token == Token::At),
                ));
                delimiters.push(b'(');
            }
            Token::LBracket => delimiters.push(b'['),
            Token::LBrace => delimiters.push(b'{'),
            Token::RParen | Token::RBracket | Token::RBrace => {
                if matches!(token, Token::RParen)
                    && anonymous_params
                        .last()
                        .is_some_and(|(depth, _)| *depth + 1 == delimiters.len())
                {
                    after_anonymous_params = anonymous_params
                        .pop()
                        .is_some_and(|(_, anonymous)| anonymous);
                }
                delimiters.pop();
            }
            _ => {}
        }
        tokens.push(Spanned {
            token,
            start,
            end: i,
        });
        scope.observe(&tokens, statement_start, &delimiters, source, i);
        if file_command {
            file_commands::lex(source, &mut i, &mut tokens, error_byte)?;
        } else if command {
            command_syntax::lex(source, &mut i, &mut tokens, error_byte, raw_command)?;
        }
        if tokens.len() > 100_000 {
            return Err("Script exceeds 100000 tokens".into());
        }
    }
    tokens.push(Spanned {
        token: Token::Eof,
        start: i,
        end: i,
    });
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
    depth: usize,
    matrix_depth: usize,
    nodes: usize,
    error_byte: usize,
    source: std::sync::Arc<str>,
    /// Token indexes where whitespace acted as a matrix element separator.
    implicit_commas: std::collections::BTreeSet<usize>,
}
impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }
    /// `int64(9007199254740993)`: the reference keeps an integer literal above 2^53
    /// exact inside `int64`/`uint64`, so the digits go to a hidden constructor.
    fn exact_integer_literal(&mut self, name: &str) -> Option<Expr> {
        if !matches!(name, "int64" | "uint64")
            || self.peek() != &Token::LParen
            || (self.matrix_depth > 0
                && self.tokens[self.pos].start != self.tokens[self.pos - 1].end)
        {
            return None;
        }
        let number = self.tokens.get(self.pos + 1)?;
        let close = self.tokens.get(self.pos + 2)?;
        let text = self.source.get(number.start..number.end)?;
        let large = matches!(number.token, Token::Number(v) if v >= 9007199254740992.);
        if !large || close.token != Token::RParen || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let text = text.to_owned();
        self.pos += 3;
        self.error_byte = self.tokens[self.pos - 1].start;
        Some(Expr::Call(
            format!("\u{1}{name}_literal"),
            vec![Expr::String(text)],
        ))
    }
    fn take(&mut self) -> Token {
        self.error_byte = self.tokens[self.pos].start;
        let t = self.peek().clone();
        if t != Token::Eof {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, token: Token) -> Result<()> {
        if *self.peek() == token {
            self.take();
            Ok(())
        } else {
            self.error_byte = self.tokens[self.pos].start;
            Err(format!("Expected {token:?}, found {:?}", self.peek()))
        }
    }
    fn bracketed_assignment(&self) -> bool {
        let mut closing = Vec::new();
        for (offset, spanned) in self.tokens[self.pos..].iter().enumerate() {
            match spanned.token {
                Token::LParen => closing.push(Token::RParen),
                Token::LBrace => closing.push(Token::RBrace),
                Token::LBracket => closing.push(Token::RBracket),
                Token::RParen | Token::RBrace | Token::RBracket => {
                    if closing.pop().as_ref() != Some(&spanned.token) {
                        return false;
                    }
                    if closing.is_empty() {
                        return self
                            .tokens
                            .get(self.pos + offset + 1)
                            .is_some_and(|t| t.token == Token::Eq);
                    }
                }
                _ => {}
            }
        }
        false
    }
    /// After a name: `@Super` written directly against it (no spaces), as in
    /// classdef `obj@Super(args)` or `obj@pkg.A(args)`. The superclass may be
    /// a package class (`pkg.A`, `a.b.A`): dots tight against the name are
    /// part of it, not a field. Returns that name.
    fn superclass_reference(&mut self) -> Option<String> {
        let at = self.tokens.get(self.pos)?;
        let base = self.tokens.get(self.pos + 1)?;
        if at.token != Token::At
            || self.pos == 0
            || at.start != self.tokens[self.pos - 1].end
            || base.start != at.end
        {
            return None;
        }
        let Token::Name(first) = &base.token else {
            return None;
        };
        let mut name = first.clone();
        let mut end = base.end;
        let mut next = self.pos + 2;
        while let (Some(dot), Some(part)) = (self.tokens.get(next), self.tokens.get(next + 1)) {
            let Token::Name(part_name) = &part.token else {
                break;
            };
            if dot.token != Token::Dot
                || dot.start != end
                || part.start != dot.end
                || keyword(part_name)
                || !crate::files::valid_name(part_name)
                || name.len() + 1 + part_name.len() > 256
                || name.bytes().filter(|b| *b == b'.').count() >= 7
            {
                break;
            }
            name.push('.');
            name.push_str(part_name);
            end = part.end;
            next += 2;
        }
        let taken = next - self.pos;
        for _ in 0..taken {
            self.take();
        }
        Some(name)
    }
    fn expr(&mut self, min: u8) -> Result<Expr> {
        self.error_byte = self.tokens[self.pos].start;
        self.nodes += 1;
        if self.nodes > 256 {
            return Err(
                "Statement exceeds 256 expression nodes; split it into smaller statements".into(),
            );
        }
        self.depth += 1;
        if self.depth > 128 {
            return Err("Expression nesting exceeds 128 levels".into());
        }
        let mut left = match self.take() {
            Token::Number(n) => Expr::Number(n),
            Token::Imaginary(n) => Expr::Imaginary(n),
            Token::String(s) => Expr::String(s),
            Token::DoubleQuoted(s) => Expr::DoubleQuoted(s),
            Token::Colon => Expr::All,
            Token::Name(name) => {
                if keyword(&name) && name != "end" {
                    return Err(format!("Keyword '{name}' is not valid in an expression"));
                }
                if let Some(literal) = self.exact_integer_literal(&name) {
                    literal
                } else if let Some(base) = self.superclass_reference() {
                    // classdef `obj@Super(args)` / `method@Super(obj, ...)`:
                    // a call named `name@Super`, which no lexer name can spell.
                    let args = if *self.peek() == Token::LParen
                        && self.tokens[self.pos].start == self.tokens[self.pos - 1].end
                    {
                        self.take();
                        self.arguments(false)?
                    } else {
                        Vec::new()
                    };
                    Expr::Call(format!("{name}@{base}"), args)
                } else if matches!(self.peek(), Token::LParen | Token::LBrace)
                    && (self.matrix_depth == 0
                        || self.tokens[self.pos].start == self.tokens[self.pos - 1].end)
                {
                    let content = self.take() == Token::LBrace;
                    let args = self.arguments(content)?;
                    if content {
                        Expr::CellContents(name, args)
                    } else {
                        Expr::Call(name, args)
                    }
                } else {
                    Expr::Name(name)
                }
            }
            Token::Plus => Expr::Unary(false, Box::new(self.expr(35)?)),
            Token::Minus => Expr::Unary(true, Box::new(self.expr(35)?)),
            Token::Ignore => Expr::Not(Box::new(self.expr(35)?)),
            Token::LParen => {
                let old = self.matrix_depth;
                self.matrix_depth = 0;
                let e = self.expr(0)?;
                self.expect(Token::RParen)?;
                self.matrix_depth = old;
                e
            }
            Token::LBracket => self.matrix()?,
            Token::LBrace => self.cell_literal()?,
            Token::At => self.function_handle()?,
            other => {
                return Err(format!("Expected an expression, found {other:?}"));
            }
        };
        loop {
            // Field access and chained indexing bind tighter than any operator,
            // but only when written directly against the previous token.
            if self.pos > 0 && self.tokens[self.pos].start == self.tokens[self.pos - 1].end {
                match self.peek() {
                    Token::Dot => {
                        self.take();
                        self.nodes += 1;
                        if self.nodes > 256 {
                            return Err("Statement exceeds 256 expression nodes".into());
                        }
                        left = match self.take() {
                            Token::Name(field) => Expr::Field(Box::new(left), field),
                            Token::LParen => {
                                let old = self.matrix_depth;
                                self.matrix_depth = 0;
                                let name = self.expr(0)?;
                                self.expect(Token::RParen)?;
                                self.matrix_depth = old;
                                Expr::DynamicField(Box::new(left), Box::new(name))
                            }
                            _ => {
                                return Err("Expected a field name after '.'".into());
                            }
                        };
                        continue;
                    }
                    Token::LParen | Token::LBrace
                        if matches!(
                            left,
                            Expr::Field(..)
                                | Expr::DynamicField(..)
                                | Expr::Index(..)
                                | Expr::Brace(..)
                                | Expr::CellContents(..)
                        ) =>
                    {
                        self.nodes += 1;
                        if self.nodes > 256 {
                            return Err("Statement exceeds 256 expression nodes".into());
                        }
                        let content = self.take() == Token::LBrace;
                        let args = self.arguments(content)?;
                        left = if content {
                            Expr::Brace(Box::new(left), args)
                        } else {
                            Expr::Index(Box::new(left), args)
                        };
                        continue;
                    }
                    _ => {}
                }
            }
            if *self.peek() == Token::LParen
                && matches!(
                    left,
                    Expr::Call(..) | Expr::CellContents(..) | Expr::Apply(..)
                )
                && self.tokens[self.pos].start == self.tokens[self.pos - 1].end
            {
                self.nodes += 1;
                if self.nodes > 256 {
                    return Err("Statement exceeds 256 expression nodes".into());
                }
                self.take();
                let old = self.matrix_depth;
                self.matrix_depth = 0;
                let mut args = Vec::new();
                if *self.peek() != Token::RParen {
                    loop {
                        args.push(self.expr(0)?);
                        if *self.peek() != Token::Comma {
                            break;
                        }
                        self.take();
                    }
                }
                self.expect(Token::RParen)?;
                self.matrix_depth = old;
                left = Expr::Apply(Box::new(left), args);
                continue;
            }
            if matches!(self.peek(), Token::Transpose | Token::NonConjugateTranspose) && min <= 40 {
                self.nodes += 1;
                if self.nodes > 256 {
                    return Err("Statement exceeds 256 expression nodes".into());
                }
                let conjugate = self.take() == Token::Transpose;
                left = if conjugate {
                    Expr::Transpose(Box::new(left))
                } else {
                    Expr::NonConjugateTranspose(Box::new(left))
                };
                continue;
            }
            let prec = match self.peek() {
                Token::ShortOr => 2,
                Token::ShortAnd => 3,
                Token::Or => 5,
                Token::And => 6,
                Token::Equal
                | Token::NotEqual
                | Token::Less
                | Token::LessEqual
                | Token::Greater
                | Token::GreaterEqual => 8,
                Token::Colon => 10,
                Token::Plus | Token::Minus => 20,
                Token::Star
                | Token::Slash
                | Token::Backslash
                | Token::DotStar
                | Token::DotSlash
                | Token::DotBackslash => 30,
                Token::Power | Token::DotPower => 40,
                _ => break,
            };
            if prec < min {
                break;
            }
            // Inside brackets, [1 -2] concatenates, whereas [1 - 2] subtracts.
            if self.matrix_depth > 0
                && matches!(self.peek(), Token::Plus | Token::Minus)
                && self.pos > 0
                && self.tokens[self.pos].start > self.tokens[self.pos - 1].end
                && self.tokens[self.pos + 1].start == self.tokens[self.pos].end
            {
                break;
            }
            let op = self.take();
            let right = self.expr(prec + 1)?;
            if op == Token::Colon {
                if *self.peek() == Token::Colon {
                    self.take();
                    let end = self.expr(11)?;
                    left = Expr::Range(Box::new(left), Some(Box::new(right)), Box::new(end));
                } else {
                    left = Expr::Range(Box::new(left), None, Box::new(right));
                }
            } else {
                left = Expr::Binary(op, Box::new(left), Box::new(right));
            }
        }
        self.depth -= 1;
        Ok(left)
    }
    /// Comma-separated arguments after an already consumed `(` or `{`.
    fn arguments(&mut self, content: bool) -> Result<Vec<Expr>> {
        let closing = if content {
            Token::RBrace
        } else {
            Token::RParen
        };
        let old = self.matrix_depth;
        self.matrix_depth = 0;
        let mut args = Vec::new();
        let mut seen_name_value = false;
        if *self.peek() != closing {
            loop {
                // Keep Name=Value distinct from a text argument until the
                // evaluator knows whether this is a call or an index.
                let name_value = !content
                    && matches!(self.peek(), Token::Name(name) if crate::files::valid_name(name) && !keyword(name))
                    && self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|t| t.token == Token::Eq);
                if seen_name_value && !name_value {
                    return Err("Positional arguments cannot follow Name=Value arguments".into());
                }
                if name_value {
                    let Token::Name(name) = self.take() else {
                        unreachable!()
                    };
                    self.take();
                    self.nodes += 1;
                    if self.nodes > 256 {
                        return Err("Statement exceeds 256 expression nodes; split it into smaller statements".into());
                    }
                    args.push(Expr::NameValue(name));
                    if *self.peek() == Token::RParen {
                        return Err("Name=Value arguments require a value".into());
                    }
                    seen_name_value = true;
                }
                args.push(self.expr(0)?);
                if *self.peek() != Token::Comma {
                    break;
                }
                self.take();
            }
        }
        self.expect(closing)?;
        self.matrix_depth = old;
        Ok(args)
    }
    /// After `@`: a named handle or an anonymous function.
    fn function_handle(&mut self) -> Result<Expr> {
        match self.peek().clone() {
            Token::Name(mut name) if !keyword(&name) => {
                self.take();
                // `@pkg.sub.name`: a handle to a package function, written
                // without spaces.
                while *self.peek() == Token::Dot
                    && self.tokens[self.pos].start == self.tokens[self.pos - 1].end
                    && let Some(Token::Name(part)) = self.tokens.get(self.pos + 1).map(|t| &t.token)
                    && !keyword(part)
                    && name.len() + part.len() < 256
                {
                    name.push('.');
                    name.push_str(part);
                    self.pos += 2;
                }
                Ok(Expr::FunctionHandle(name))
            }
            Token::LParen => {
                self.take();
                let mut params: Vec<String> = Vec::new();
                while *self.peek() != Token::RParen {
                    match self.take() {
                        Token::Name(n) if !keyword(&n) && !params.contains(&n) => params.push(n),
                        Token::Ignore => params.push("~".into()),
                        _ => {
                            return Err(
                                "Anonymous function parameters must be distinct names or ~".into(),
                            );
                        }
                    }
                    if params.len() > 32 {
                        return Err("An anonymous function supports at most 32 inputs".into());
                    }
                    if *self.peek() == Token::Comma {
                        self.take();
                        if *self.peek() == Token::RParen {
                            return Err("Expected a parameter name after comma".into());
                        }
                    } else if *self.peek() != Token::RParen {
                        self.error_byte = self.tokens[self.pos].start;
                        return Err("Expected a comma or ')' in the parameter list".into());
                    }
                }
                self.expect(Token::RParen)?;
                let body_start = self.pos;
                let body = self.expr(0)?;
                let mut text = String::from("@(");
                text.push_str(&params.join(","));
                text.push(')');
                for index in body_start..self.pos {
                    if self.implicit_commas.contains(&index) {
                        text.push(',');
                    }
                    let token = &self.tokens[index];
                    if token.token == Token::Newline {
                        text.push(';');
                    } else {
                        text.push_str(&self.source[token.start..token.end]);
                    }
                }
                if text.len() > 4096 {
                    return Err("Anonymous function text exceeds 4096 bytes".into());
                }
                Ok(Expr::Anonymous(std::sync::Arc::new(AnonymousFunction {
                    params,
                    body,
                    text,
                })))
            }
            _ => {
                // Point at the '@' itself, as the editor diagnostics expect.
                self.error_byte = self.tokens[self.pos - 1].start;
                Err("Expected a function name or '(' after '@'".into())
            }
        }
    }
    fn matrix_rows(&mut self, close: Token) -> Result<Vec<Vec<Expr>>> {
        self.matrix_depth += 1;
        let result = (|| {
            let mut rows = Vec::new();
            let mut row = Vec::new();
            while matches!(self.peek(), Token::Newline | Token::Semi) {
                self.take();
            }
            if *self.peek() == Token::Comma {
                self.take();
                while matches!(self.peek(), Token::Newline | Token::Semi) {
                    self.take();
                }
            }
            if *self.peek() == close {
                self.take();
                return Ok(rows);
            }
            loop {
                row.push(self.expr(0)?);
                if *self.peek() == close {
                    self.take();
                    rows.push(row);
                    break;
                }
                match self.peek() {
                    Token::Comma => {
                        self.take();
                        while matches!(self.peek(), Token::Newline) {
                            self.take();
                        }
                        if *self.peek() == close {
                            self.take();
                            rows.push(row);
                            break;
                        }
                        if *self.peek() == Token::Semi {
                            self.take();
                            rows.push(row);
                            row = Vec::new();
                            while matches!(self.peek(), Token::Newline | Token::Semi) {
                                self.take();
                            }
                            if *self.peek() == close {
                                self.take();
                                break;
                            }
                        }
                    }
                    Token::Semi | Token::Newline => {
                        self.take();
                        rows.push(row);
                        row = Vec::new();
                        while matches!(self.peek(), Token::Newline | Token::Semi) {
                            self.take();
                        }
                        if *self.peek() == close {
                            self.take();
                            break;
                        }
                    }
                    _ => {
                        if self.tokens[self.pos].start == self.tokens[self.pos - 1].end {
                            self.error_byte = self.tokens[self.pos].start;
                            return Err("Expected a matrix separator".into());
                        }
                        self.implicit_commas.insert(self.pos);
                    }
                }
            }
            Ok(rows)
        })();
        self.matrix_depth -= 1;
        result
    }
    fn cell_literal(&mut self) -> Result<Expr> {
        Ok(Expr::CellLiteral(self.matrix_rows(Token::RBrace)?))
    }
    fn matrix(&mut self) -> Result<Expr> {
        Ok(Expr::Matrix(self.matrix_rows(Token::RBracket)?))
    }
}
/// Parse exactly one expression (used by `str2func`).
pub fn parse_expression(source: &str) -> Result<Expr> {
    if source.len() > 65536 {
        return Err("Expression text exceeds 65536 bytes".into());
    }
    let mut byte = 0;
    let tokens = lex_in(source, &mut byte, command_syntax::Scope::new(false, None))?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
        matrix_depth: 0,
        nodes: 0,
        error_byte: 0,
        source: source.into(),
        implicit_commas: Default::default(),
    };
    let expr = p.expr(0)?;
    while matches!(p.peek(), Token::Newline | Token::Semi) {
        p.take();
    }
    if *p.peek() != Token::Eof {
        return Err("Unexpected text after the expression".into());
    }
    Ok(expr)
}
pub fn parse(source: &str) -> Result<Vec<Statement>> {
    parse_detailed(source).map_err(|e| e.to_string())
}
/// Parse a script or function file: command syntax (`myfn a b`) applies to
/// names not assigned earlier in the same code unit.
pub fn parse_detailed(source: &str) -> std::result::Result<Vec<Statement>, Diagnostic> {
    parse_scoped(source, None)
}
/// Parse Command Window, -batch or eval text: names `is_variable` accepts
/// (the current workspace) are variables too, so `x -1` subtracts when `x`
/// exists and calls `x('-1')` otherwise.
pub fn parse_in_workspace(
    source: &str,
    is_variable: &dyn Fn(&str) -> bool,
) -> std::result::Result<Vec<Statement>, Diagnostic> {
    parse_scoped(source, Some(is_variable))
}
fn parse_scoped(
    source: &str,
    workspace: Option<&dyn Fn(&str) -> bool>,
) -> std::result::Result<Vec<Statement>, Diagnostic> {
    if source.len() > 1_000_000 {
        return Err(Diagnostic::new(source, 0, "Script exceeds 1 MB".into()));
    }
    let mut byte = 0;
    let tokens = lex_in(
        source,
        &mut byte,
        command_syntax::Scope::new(true, workspace),
    )
    .map_err(|message| Diagnostic::new(source, byte, message))?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
        matrix_depth: 0,
        nodes: 0,
        error_byte: 0,
        source: source.into(),
        implicit_commas: Default::default(),
    };
    let function_file = p
        .tokens
        .iter()
        .find(|t| !is_separator(&t.token))
        .is_some_and(|t| matches!(&t.token, Token::Name(n) if n == "function"));
    let statements = function_decl::parse_file(&mut p, function_file);
    let result = statements.and_then(|statements| {
        if *p.peek() != Token::Eof {
            p.error_byte = p.tokens[p.pos].start;
            Err(format!("Unexpected block keyword {:?}", p.peek()))
        } else {
            function_decl::validate_program(&statements)?;
            Ok(statements)
        }
    });
    result.map_err(|message| Diagnostic::new(source, p.error_byte, message))
}
/// Flatten `s(2).a{3}` into its root variable and accessor chain.
fn assignment_path(mut expr: Expr) -> Result<(String, Vec<PathStep>)> {
    let mut steps = Vec::new();
    let root = loop {
        let subscripts = |args: &[Expr]| {
            if args.iter().any(|arg| matches!(arg, Expr::NameValue(_))) {
                return Err("Name=Value syntax is not supported for indexing".to_owned());
            }
            if args.is_empty() || args.len() > 32 {
                Err("Indexed assignment requires one or two subscripts".to_owned())
            } else {
                Ok(())
            }
        };
        expr = match expr {
            Expr::Field(base, name) => {
                steps.push(PathStep::Field(name));
                *base
            }
            Expr::DynamicField(base, name) => {
                steps.push(PathStep::Dynamic(*name));
                *base
            }
            Expr::Index(base, args) => {
                subscripts(&args)?;
                steps.push(PathStep::Paren(args));
                *base
            }
            Expr::Brace(base, args) => {
                subscripts(&args)?;
                steps.push(PathStep::Brace(args));
                *base
            }
            Expr::Call(name, args) => {
                subscripts(&args)?;
                steps.push(PathStep::Paren(args));
                break name;
            }
            Expr::CellContents(name, args) => {
                subscripts(&args)?;
                steps.push(PathStep::Brace(args));
                break name;
            }
            Expr::Name(name) => break name,
            _ => return Err("Assignment requires a variable name or indexed variable".into()),
        };
    };
    steps.reverse();
    Ok((root, steps))
}
fn is_separator(token: &Token) -> bool {
    matches!(token, Token::Newline | Token::Semi | Token::Comma)
}
fn separator(p: &mut Parser, eof: bool) -> Result<()> {
    if is_separator(p.peek()) || (eof && *p.peek() == Token::Eof) {
        Ok(())
    } else {
        p.error_byte = p.tokens[p.pos].start;
        Err("Expected a statement separator".into())
    }
}
fn ends_block(name: &str) -> bool {
    matches!(
        name,
        "end" | "elseif" | "else" | "catch" | "case" | "otherwise"
    )
}
fn parse_condition(p: &mut Parser) -> Result<Expr> {
    let condition = p.expr(0)?;
    if p.tokens[p.pos - 1].token != Token::RParen
        || *p.peek() == Token::Eof
        || matches!(p.peek(), Token::Name(name) if ends_block(name))
    {
        separator(p, false)?;
    }
    Ok(condition)
}
fn end_block(p: &mut Parser) -> Result<()> {
    if matches!(p.peek(), Token::Name(n) if n == "end") {
        p.take();
        separator(p, true)
    } else {
        p.error_byte = p.tokens[p.pos].start;
        Err("Expected 'end' to close the block".into())
    }
}
fn parse_block(p: &mut Parser, depth: usize, in_loop: bool) -> Result<Vec<Statement>> {
    if depth > 64 {
        return Err("Control-flow nesting exceeds 64 blocks".into());
    }
    let mut statements = Vec::new();
    while *p.peek() != Token::Eof {
        if matches!(p.peek(), Token::Newline | Token::Semi | Token::Comma) {
            p.take();
            continue;
        }
        p.nodes = 0;
        let start = p.tokens[p.pos].start;
        p.error_byte = start;
        if let Token::Name(name) = p.peek().clone() {
            if name == "function" && depth == 0 {
                break;
            }
            if ends_block(&name) {
                break;
            }
            let kind = match name.as_str() {
                "global" | "persistent" => {
                    p.take();
                    let mut names = Vec::new();
                    while let Token::Name(n) = p.peek().clone() {
                        if !crate::files::valid_name(&n) || keyword(&n) {
                            return Err("Invalid variable name in declaration.".into());
                        }
                        p.take();
                        names.push(n);
                        if names.len() > 255 {
                            return Err("Declaration exceeds 255 names".into());
                        }
                    }
                    separator(p, true)?;
                    Some(StatementKind::Declaration {
                        persistent: name == "persistent",
                        names,
                    })
                }
                "function" => {
                    return Err(
                        "Function definitions cannot appear inside a control-flow block".into(),
                    );
                }
                "if" => {
                    p.take();
                    let mut branches = Vec::new();
                    let mut byte = start;
                    loop {
                        p.nodes = 0;
                        let condition = parse_condition(p)?;
                        let body = parse_block(p, depth + 1, in_loop)?;
                        branches.push(ConditionalBranch {
                            condition,
                            body,
                            byte,
                        });
                        if !matches!(p.peek(), Token::Name(n) if n == "elseif") {
                            break;
                        }
                        byte = p.tokens[p.pos].start;
                        p.take();
                    }
                    let otherwise = if matches!(p.peek(), Token::Name(n) if n == "else") {
                        p.take();
                        // A statement may follow `else` on its line (`else x = 1`), as after
                        // `otherwise` and `try`. MATLAB's "else if" is therefore a nested if
                        // and needs two ends.
                        parse_block(p, depth + 1, in_loop)?
                    } else {
                        Vec::new()
                    };
                    end_block(p)?;
                    Some(StatementKind::If {
                        branches,
                        otherwise,
                    })
                }
                "switch" => {
                    p.take();
                    let subject = p.expr(0)?;
                    separator(p, false)?;
                    while matches!(p.peek(), Token::Newline | Token::Semi | Token::Comma) {
                        p.take();
                    }
                    let mut cases = Vec::new();
                    while matches!(p.peek(), Token::Name(n) if n == "case") {
                        let byte = p.tokens[p.pos].start;
                        p.take();
                        p.nodes = 0;
                        let value = p.expr(0)?;
                        separator(p, false)?;
                        let body = parse_block(p, depth + 1, in_loop)?;
                        cases.push(SwitchCase { value, body, byte });
                    }
                    let otherwise = if matches!(p.peek(), Token::Name(n) if n == "otherwise") {
                        p.take();
                        parse_block(p, depth + 1, in_loop)?
                    } else {
                        Vec::new()
                    };
                    if !matches!(p.peek(), Token::Name(n) if n == "end") {
                        p.error_byte = p.tokens[p.pos].start;
                        return Err(
                            "Expected 'case', 'otherwise' or 'end' in the switch block".into()
                        );
                    }
                    end_block(p)?;
                    Some(StatementKind::Switch {
                        subject,
                        cases,
                        otherwise,
                    })
                }
                "while" => {
                    p.take();
                    let condition = parse_condition(p)?;
                    let body = parse_block(p, depth + 1, true)?;
                    end_block(p)?;
                    Some(StatementKind::While { condition, body })
                }
                "for" | "parfor" => {
                    p.take();
                    // `parfor (k = range, M)`: the worker limit is evaluated and ignored,
                    // since MatRust runs parfor bodies serially in loop order.
                    let paren = name == "parfor" && matches!(p.peek(), Token::LParen);
                    if paren {
                        p.take();
                    }
                    let name = match p.take() {
                        Token::Name(n) if !keyword(&n) => n,
                        _ => return Err("A for loop requires a variable name".into()),
                    };
                    p.expect(Token::Eq)?;
                    let values = p.expr(0)?;
                    let mut arguments = Vec::new();
                    if paren {
                        if matches!(p.peek(), Token::Comma) {
                            p.take();
                            arguments.push(p.expr(0)?);
                        }
                        p.expect(Token::RParen)?;
                    }
                    separator(p, false)?;
                    let body = parse_block(p, depth + 1, true)?;
                    end_block(p)?;
                    let kind = StatementKind::For { name, values, body };
                    Some(if arguments.is_empty() {
                        kind
                    } else {
                        StatementKind::SerialBlock {
                            arguments,
                            body: vec![Statement {
                                kind,
                                span: start..p.tokens[p.pos.saturating_sub(1)].end,
                            }],
                        }
                    })
                }
                "spmd" => {
                    // One serial worker: the body runs once, like `if true`.
                    p.take();
                    let mut arguments = Vec::new();
                    if matches!(p.peek(), Token::LParen) {
                        p.take();
                        arguments.push(p.expr(0)?);
                        if matches!(p.peek(), Token::Comma) {
                            p.take();
                            arguments.push(p.expr(0)?);
                        }
                        p.expect(Token::RParen)?;
                    }
                    separator(p, false)?;
                    let body = parse_block(p, depth + 1, in_loop)?;
                    end_block(p)?;
                    Some(StatementKind::SerialBlock { arguments, body })
                }
                "try" => {
                    p.take();
                    let body = parse_block(p, depth + 1, in_loop)?;
                    let (error, handler) = if matches!(p.peek(), Token::Name(n) if n == "catch") {
                        p.take();
                        // `catch err` names the error only on the same line, and only when
                        // the identifier is not the start of a statement such as `catch f(x)`.
                        let error =
                            match (p.peek().clone(), p.tokens.get(p.pos + 1).map(|t| &t.token)) {
                                (
                                    Token::Name(n),
                                    Some(Token::Newline | Token::Semi | Token::Comma | Token::Eof),
                                ) if !keyword(&n) => {
                                    p.take();
                                    Some(n)
                                }
                                _ => None,
                            };
                        (error, parse_block(p, depth + 1, in_loop)?)
                    } else {
                        (None, Vec::new())
                    };
                    end_block(p)?;
                    Some(StatementKind::Try {
                        body,
                        error,
                        handler,
                    })
                }
                "break" | "continue" | "return" => {
                    p.take();
                    if name != "return" && !in_loop {
                        return Err(format!("'{name}' requires an enclosing loop"));
                    }
                    separator(p, true)?;
                    Some(match name.as_str() {
                        "break" => StatementKind::Break,
                        "continue" => StatementKind::Continue,
                        _ => StatementKind::Return,
                    })
                }
                _ if keyword(&name) => return Err(format!("'{name}' syntax is not supported yet")),
                _ => None,
            };
            if let Some(kind) = kind {
                let end = p.tokens[p.pos.saturating_sub(1)].end;
                statements.push(Statement {
                    kind,
                    span: start..end,
                });
                continue;
            }
        }
        let mut list_target = None;
        let targets = if matches!(p.peek(), Token::Name(_))
            && p.tokens
                .get(p.pos + 1)
                .is_some_and(|t| t.token == Token::Eq)
        {
            let Token::Name(n) = p.take() else {
                unreachable!()
            };
            p.take();
            vec![Some(n)]
        } else if *p.peek() == Token::LBracket && p.bracketed_assignment() {
            p.take();
            let mut destinations = Vec::new();
            loop {
                let target = match p.peek() {
                    Token::Name(n) if !keyword(n) => {
                        if p.tokens.get(p.pos + 1).is_some_and(|t| {
                            matches!(t.token, Token::Dot | Token::LBrace | Token::LParen)
                        }) {
                            let expr = p.expr(0)?;
                            assignment_path(expr.clone())?;
                            Some(expr)
                        } else {
                            let Token::Name(name) = p.take() else {
                                unreachable!()
                            };
                            Some(Expr::Name(name))
                        }
                    }
                    Token::Ignore => {
                        p.take();
                        None
                    }
                    _ => return Err("Expected a variable name or ~ in output list".into()),
                };
                destinations.push(target);
                if destinations.len() > 32 {
                    return Err("At most 32 outputs can be assigned in one statement".into());
                }
                if *p.peek() == Token::RBracket {
                    break;
                }
                if *p.peek() == Token::Comma {
                    p.take();
                } else if !matches!(p.peek(), Token::Name(_) | Token::Ignore)
                    || p.tokens[p.pos].start == p.tokens[p.pos - 1].end
                {
                    p.error_byte = p.tokens[p.pos].start;
                    return Err("Expected a comma or space between output names".into());
                }
            }
            p.expect(Token::RBracket)?;
            p.expect(Token::Eq)?;
            if destinations
                .iter()
                .all(|target| matches!(target, None | Some(Expr::Name(_))))
            {
                destinations
                    .into_iter()
                    .map(|target| match target {
                        Some(Expr::Name(name)) => Some(name),
                        _ => None,
                    })
                    .collect()
            } else {
                list_target = Some(destinations);
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let starts_with_name = matches!(p.peek(), Token::Name(_));
        let mut expr = p.expr(0)?;
        let mut cell_content_target = false;
        let mut path_target = None;
        let indexed_target = if targets.is_empty()
            && list_target.is_none()
            && *p.peek() == Token::Eq
            && matches!(
                expr,
                Expr::Field(..) | Expr::DynamicField(..) | Expr::Index(..) | Expr::Brace(..)
            ) {
            if !starts_with_name {
                return Err("An assignment target must begin with a variable name".into());
            }
            path_target = Some(assignment_path(std::mem::replace(&mut expr, Expr::All))?);
            p.take();
            expr = p.expr(0)?;
            None
        } else if targets.is_empty() && list_target.is_none() && *p.peek() == Token::Eq {
            if !starts_with_name {
                return Err("An indexed assignment target must begin with a variable name".into());
            }
            let (name, args) = match expr {
                Expr::Call(name, args) => (name, args),
                Expr::CellContents(name, args) => {
                    cell_content_target = true;
                    (name, args)
                }
                _ => return Err("Assignment requires a variable name or indexed variable".into()),
            };
            if args.iter().any(|arg| matches!(arg, Expr::NameValue(_))) {
                return Err("Name=Value syntax is not supported for indexing".into());
            }
            if args.is_empty() || args.len() > 32 {
                return Err("Indexed assignment requires one or two subscripts".into());
            }
            p.take();
            expr = p.expr(0)?;
            Some((name, args))
        } else {
            None
        };
        let quiet = *p.peek() == Token::Semi;
        if !matches!(
            p.peek(),
            Token::Newline | Token::Semi | Token::Comma | Token::Eof
        ) {
            p.error_byte = p.tokens[p.pos].start;
            return Err("Expected a statement separator".into());
        }
        let end = p.tokens[p.pos.saturating_sub(1)].end;
        statements.push(Statement {
            kind: StatementKind::Evaluate(Evaluation {
                targets,
                indexed_target,
                cell_content_target,
                path_target,
                list_target,
                expr,
                quiet,
            }),
            span: start..end,
        });
    }
    Ok(statements)
}

const BLOCK_OPENERS: [&str; 8] = [
    "if", "for", "while", "switch", "try", "function", "parfor", "spmd",
];

/// Block openers minus `end`s at statement starts outside brackets, for input
/// the parser stops at because it does not support the block yet (`switch`).
fn open_blocks(tokens: &[Spanned]) -> isize {
    let (mut blocks, mut nesting) = (0isize, 0usize);
    let mut statement_start = true;
    let mut after_else = false;
    for spanned in tokens {
        match &spanned.token {
            Token::LParen | Token::LBracket | Token::LBrace => nesting += 1,
            Token::RParen | Token::RBracket | Token::RBrace => nesting = nesting.saturating_sub(1),
            Token::Name(name) if nesting == 0 && (statement_start || after_else) => {
                match name.as_str() {
                    name if BLOCK_OPENERS.contains(&name) => blocks += 1,
                    "end" if statement_start => blocks -= 1,
                    _ => {}
                }
            }
            _ => {}
        }
        after_else = matches!(&spanned.token, Token::Name(n) if n == "else") && statement_start;
        statement_start =
            nesting == 0 && matches!(spanned.token, Token::Newline | Token::Semi | Token::Comma);
    }
    blocks
}

/// True when `source` is unfinished input that more lines could complete: an
/// open block without its `end`, an unclosed bracket, a trailing `...` or an
/// unclosed `%{` block comment. The input is parsed with its open brackets
/// closed and the line break Enter adds. Its first error must lie in that
/// added text, at the end of input or at an unclosed bracket (`x [` with a
/// variable `x` waits for the bracket's contents); an earlier error is real,
/// so the input counts as complete and reports it. Command syntax is decided
/// lexically, as in script files; see [`is_incomplete_in_workspace`].
pub fn is_incomplete(source: &str) -> bool {
    incomplete(source, None)
}
/// [`is_incomplete`] for text run by [`parse_in_workspace`] with the same
/// `is_variable`: with a workspace variable `x`, `x [` is an expression whose
/// bracket is open (more lines follow); without one it is the complete
/// command `x('[')`.
pub fn is_incomplete_in_workspace(source: &str, is_variable: &dyn Fn(&str) -> bool) -> bool {
    incomplete(source, Some(is_variable))
}
fn incomplete(source: &str, workspace: Option<&dyn Fn(&str) -> bool>) -> bool {
    let scope = || command_syntax::Scope::new(true, workspace);
    let mut byte = 0;
    let Ok(tokens) = lex_in(source, &mut byte, scope()) else {
        return false;
    };
    // Closers of the brackets still open at the end, with each opener's byte.
    let mut open = Vec::new();
    for spanned in &tokens {
        match spanned.token {
            Token::LParen => open.push((Token::RParen, spanned.start)),
            Token::LBracket => open.push((Token::RBracket, spanned.start)),
            Token::LBrace => open.push((Token::RBrace, spanned.start)),
            Token::RParen | Token::RBracket | Token::RBrace
                if open.pop().map(|(closer, _)| closer).as_ref() != Some(&spanned.token) =>
            {
                return false;
            }
            _ => {}
        }
    }
    let closers: String = open
        .iter()
        .rev()
        .map(|(token, _)| match token {
            Token::RParen => ')',
            Token::RBracket => ']',
            _ => '}',
        })
        .collect();
    // Only blanks, comments and continuations follow the last real token.
    let body = &source[..tokens.iter().rev().nth(1).map_or(0, |last| last.end)];
    // A `...` on the last line swallows the line break Enter adds.
    // An unclosed `%{` block comment swallows it too.
    let continued = lex_in(&format!("{source}\n"), &mut byte, scope()).is_ok_and(|tokens| {
        !tokens
            .iter()
            .any(|t| t.token == Token::Newline && t.end == source.len() + 1)
    });
    let line_break = body.len() + closers.len();
    match parse_scoped(&format!("{body}{closers}\n"), workspace) {
        Ok(statements) => {
            !closers.is_empty()
                || continued
                || statements
                    .iter()
                    .any(|s| matches!(&s.kind, StatementKind::Function(f) if !f.explicit_end))
        }
        // A bracket that cannot start here (`x [` for a variable `x`) still
        // waits for its contents and closer, as other open brackets do.
        Err(d) if open.iter().any(|(_, start)| *start == d.byte) => true,
        // The parser stops at a block it does not support yet: count blocks.
        Err(d) if d.byte < body.len() => {
            tokens.iter().any(|t| {
                t.start == d.byte
                    && matches!(&t.token, Token::Name(n) if BLOCK_OPENERS.contains(&n.as_str()))
            }) && (open_blocks(&tokens) > 0 || !closers.is_empty() || continued)
        }
        // Enter ends the last statement unless a bracket or `...` carries it on.
        Err(d) if d.byte == line_break => !closers.is_empty() || continued,
        Err(_) => true,
    }
}

#[cfg(test)]
mod incomplete_tests {
    use super::is_incomplete;

    #[test]
    fn open_blocks_are_incomplete() {
        for src in [
            "if 1",
            "if 1\n disp(5)",
            "for k=1:3",
            "while true\n x=1;",
            "switch 2\n case 2",
            "try\n x=1;",
            "try\n error('a')\ncatch",
            "if 1\n x=1\nelse",
            "if 1\n x=1\nelseif 2",
            "parfor k=1:3",
            "function y = f(x)\n y = x;",
            "for i=1:2\n if i\n  x=1;\n end",
        ] {
            assert!(is_incomplete(src), "{src:?}");
        }
    }

    #[test]
    fn closed_blocks_and_plain_input_are_complete() {
        for src in [
            "",
            "x = 1",
            "if 1, x=1, end",
            "if 1\n disp(5)\nend",
            "for i=1:2\n if i\n  x=1;\n end\nend\n",
            "switch 1\n case 1\n  x=1;\nend",
            "try, error('a'), catch, end",
            "function y = f(x)\n y = x;\nend",
            "x = [1 2; 3 4]\n",
        ] {
            assert!(!is_incomplete(src), "{src:?}");
        }
    }

    #[test]
    fn brackets_and_continuation_are_incomplete() {
        for src in [
            "x = (1 +",
            "x = [1 2",
            "x = [1 2;\n 3 4",
            "c = {1,",
            "f(1, [2 3",
            "x = 1 + ...",
            "x = 1 ...",
            "x = 1 + ... comment",
            "x = [1 2 ...",
            "c = {1, ...\n 2",
        ] {
            assert!(is_incomplete(src), "{src:?}");
        }
        // The `...` continued onto an empty last line, which Enter ends.
        for src in [
            "x = (1 + 2)",
            "x = [1 2\n3 4]",
            "x = 1 + ...\n 2",
            "x = 1 + ...\n",
            "x = 1 + ... comment\n  ",
        ] {
            assert!(!is_incomplete(src), "{src:?}");
        }
    }

    #[test]
    fn strings_and_comments_do_not_count() {
        for src in [
            "disp('if')",
            "disp(\"for\")",
            "x = 1 % for",
            "x = 1 % (",
            "x = 'a(' ",
            "% if\n",
            "x = 1 % ...",
            "s = 'end...'",
        ] {
            assert!(!is_incomplete(src), "{src:?}");
        }
        assert!(is_incomplete("if 1 % end"));
        assert!(is_incomplete("if 1\nelse if 2\nend"));
        assert!(!is_incomplete("if 1\nelse if 2\nend\nend"));
        assert!(is_incomplete("if 1\n disp('end')"));
    }

    #[test]
    fn end_as_index_does_not_close_or_open() {
        assert!(!is_incomplete("x = 1:5; y = x(end)"));
        assert!(!is_incomplete("x(end+1) = 3"));
        assert!(is_incomplete("if 1\n y = x(end)"));
        assert!(!is_incomplete("if 1\n y = x(end)\nend"));
    }

    #[test]
    fn real_syntax_errors_are_complete() {
        for src in [
            "x = 'abc",
            "x = 1 $ 2",
            "x = (1 + ]",
            "x = 1)",
            "end",
            "x = = 2",
            "if 1\n x=1\nend\nend",
            "else",
            "x = 1 +",
            "for k=",
        ] {
            assert!(!is_incomplete(src), "{src:?}");
        }
    }

    #[test]
    fn errors_before_the_end_are_complete_inside_blocks_and_brackets() {
        for src in [
            "if 1\nx = = 2",
            "if 1\n x = = 2\n",
            "for i=1:2\n if i\n  x = = 1",
            "if 1\n x = 1 +",
            "x = = (",
            "x = (1 + = 2",
            "x = = [1 2",
            "x = = 2 ...",
            "x = = 2\nswitch 1",
        ] {
            assert!(!is_incomplete(src), "{src:?}");
        }
    }
}
