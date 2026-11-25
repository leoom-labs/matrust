//! `classdef` files (value and handle classes, inheritance, property access
//! methods). Only a file whose first statement
//! is `classdef` reaches this parser; scripts and the console keep rejecting the
//! keyword. `properties`, `methods`, `events` and `enumeration` are keywords
//! only at the top of a classdef body, so elsewhere they stay ordinary names.
use super::*;
use std::collections::BTreeSet;

/// Who may read or write a property, or call a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Public,
    Protected,
    Private,
}
impl Access {
    pub fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Protected => "protected",
            Self::Private => "private",
        }
    }
}
#[derive(Clone, Debug)]
pub struct PropertyAttributes {
    pub get_access: Access,
    pub set_access: Access,
    pub constant: bool,
    /// No storage: the value comes from a `get.Name` method.
    pub dependent: bool,
    /// Not listed by properties/fieldnames, not displayed; still accessible.
    pub hidden: bool,
    /// Accepted; no effect until objects can be saved to MAT files.
    pub transient: bool,
    /// `SetAccess = immutable`: set only by the defining class's constructor.
    pub immutable: bool,
}
#[derive(Clone, Debug)]
pub struct Property {
    pub name: String,
    pub validation: Option<ArgumentSpec>,
    /// `name = expr`: evaluated once per run when the class is first loaded.
    pub default: Option<Expr>,
    /// Source bytes of the property line (error locations).
    pub span: std::ops::Range<usize>,
}
#[derive(Clone, Debug)]
pub struct PropertyBlock {
    pub attributes: PropertyAttributes,
    pub properties: Vec<Property>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct MethodAttributes {
    pub is_static: bool,
    pub access: Access,
    /// `methods (Abstract)`: signatures only, listed in `MethodBlock::signatures`.
    pub is_abstract: bool,
}
#[derive(Clone, Debug)]
pub struct MethodBlock {
    pub attributes: MethodAttributes,
    /// `StatementKind::Function` statements with an explicit `end`.
    pub methods: Vec<Statement>,
    /// Names declared without a body, with their bytes: the abstract methods
    /// of a `methods (Abstract)` block; in any other block, methods kept in
    /// their own files of the class folder (`@Name/method.m`).
    pub signatures: Vec<(String, usize)>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct EnumMember {
    pub name: String,
    pub args: Vec<Expr>,
    pub byte: usize,
}
#[derive(Clone, Debug)]
pub struct ClassDefinition {
    pub name: String,
    /// Accepted class attributes (only `Sealed`: the class cannot be subclassed).
    pub attributes: Vec<String>,
    /// `classdef Name < A & B`: direct superclasses in declaration order
    /// (`handle` included), with the byte of each name. A package class is
    /// the full dotted name (`pkg.A`, `a.b.A`).
    pub superclasses: Vec<(String, usize)>,
    pub properties: Vec<PropertyBlock>,
    pub methods: Vec<MethodBlock>,
    /// `events` block names in order, with their bytes.
    pub events: Vec<(String, usize)>,
    /// `enumeration` members in order: name, constructor arguments, byte.
    pub enumeration: Vec<EnumMember>,
    /// Local functions after the classdef `end`: visible to the methods only.
    pub functions: Vec<Statement>,
    /// Byte of the `classdef` keyword and of the end of its header line.
    pub span: std::ops::Range<usize>,
}

/// Why a class file was rejected.
#[derive(Clone, Debug)]
pub enum ClassFileError {
    /// A feature planned for a later part, with its verb ("handle classes
    /// are"); `byte` locates its line.
    Unsupported {
        feature: String,
        byte: usize,
    },
    Syntax(Diagnostic),
}
impl ClassFileError {
    /// Text naming the file: `<file>: <feature> is not supported yet (line N)`.
    pub fn message(&self, file: &str, source: &str) -> String {
        match self {
            Self::Unsupported { feature, byte } => {
                let line = Diagnostic::new(source, *byte, String::new()).line;
                format!("{file}: {feature} not supported yet (line {line})")
            }
            Self::Syntax(d) => format!("{file}: {d}"),
        }
    }
}

/// True when the first statement of `source` is `classdef` (blank lines and
/// `%` comments may come first). A cheap text check: no lexing.
pub fn is_class_file(source: &str) -> bool {
    let mut rest = source.strip_prefix('\u{feff}').unwrap_or(source);
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
        if rest.starts_with('%') {
            rest = rest.find(['\n', '\r']).map_or("", |at| &rest[at..]);
            continue;
        }
        break;
    }
    rest.strip_prefix("classdef")
        .is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
}

/// Parse a class file. The caller checked [`is_class_file`].
pub fn parse_class_file(source: &str) -> std::result::Result<ClassDefinition, ClassFileError> {
    if source.len() > 1_000_000 {
        return Err(ClassFileError::Syntax(Diagnostic::new(
            source,
            0,
            "Class file exceeds 1 MB".into(),
        )));
    }
    let mut byte = 0;
    let tokens = lex(source, &mut byte)
        .map_err(|message| ClassFileError::Syntax(Diagnostic::new(source, byte, message)))?;
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
    class_definition(&mut p).map_err(|e| match e {
        Failure::Unsupported(feature, byte) => ClassFileError::Unsupported { feature, byte },
        Failure::Syntax(message) => {
            ClassFileError::Syntax(Diagnostic::new(source, p.error_byte, message))
        }
    })
}

enum Failure {
    Unsupported(String, usize),
    Syntax(String),
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Syntax(message)
    }
}
type Parsed<T> = std::result::Result<T, Failure>;

fn unsupported<T>(feature: impl Into<String>, byte: usize) -> Parsed<T> {
    Err(Failure::Unsupported(feature.into(), byte))
}
fn skip_separators(p: &mut Parser) {
    while matches!(p.peek(), Token::Newline | Token::Semi | Token::Comma) {
        p.take();
    }
}
fn is_name(p: &Parser, text: &str) -> bool {
    matches!(p.peek(), Token::Name(n) if n == text)
}
fn byte(p: &Parser) -> usize {
    p.tokens[p.pos].start
}
fn syntax<T>(p: &mut Parser, message: &str) -> Parsed<T> {
    p.error_byte = byte(p);
    Err(Failure::Syntax(message.into()))
}
fn identifier(p: &mut Parser, what: &str) -> Parsed<String> {
    match p.peek().clone() {
        Token::Name(n) if crate::files::valid_name(&n) && !keyword(&n) => {
            p.take();
            Ok(n)
        }
        _ => syntax(p, &format!("Expected {what}")),
    }
}

/// One `(Name, Name = value, ...)` attribute list; the value is a name, a
/// quoted word, `true` or `false`. Returns (name, value, byte) triples.
fn attribute_list(p: &mut Parser) -> Parsed<Vec<(String, Option<String>, usize)>> {
    let mut list = Vec::new();
    if *p.peek() != Token::LParen {
        return Ok(list);
    }
    p.take();
    loop {
        let at = byte(p);
        if *p.peek() == Token::Ignore {
            // `~Static` negates a flag: the reference treats it as `= false`.
            p.take();
            let name = identifier(p, "an attribute name")?;
            list.push((name, Some("false".into()), at));
        } else {
            let name = identifier(p, "an attribute name")?;
            let value = if *p.peek() == Token::Eq {
                p.take();
                match p.take() {
                    Token::Name(v) | Token::String(v) | Token::DoubleQuoted(v) => Some(v),
                    _ => {
                        p.error_byte = p.tokens[p.pos.saturating_sub(1)].start;
                        return Err(Failure::Syntax(format!(
                            "Expected a value for the attribute '{name}'"
                        )));
                    }
                }
            } else {
                None
            };
            list.push((name, value, at));
        }
        if list.len() > 32 {
            return syntax(p, "An attribute list supports at most 32 attributes");
        }
        match p.peek() {
            Token::Comma => {
                p.take();
            }
            Token::RParen => {
                p.take();
                return Ok(list);
            }
            _ => return syntax(p, "Expected ',' or ')' in the attribute list"),
        }
    }
}
fn flag(name: &str, value: &Option<String>, at: usize) -> Parsed<bool> {
    match value.as_deref() {
        None | Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(Failure::Syntax(format!(
            "Attribute '{name}' must be true or false, not '{other}' (byte {at})"
        ))),
    }
}
fn access(name: &str, value: &Option<String>, at: usize) -> Parsed<Access> {
    match value.as_deref() {
        Some("public") => Ok(Access::Public),
        Some("protected") => Ok(Access::Protected),
        Some("private") => Ok(Access::Private),
        Some(other) => unsupported(format!("the attribute value {name} = {other} is"), at),
        None => Err(Failure::Syntax(format!("Attribute '{name}' needs a value"))),
    }
}

/// A superclass after `<` or `&`: `A`, `handle`, `pkg.A` or `a.b.A`
/// (at most eight parts, the same limit as a package-qualified name).
fn superclass_name(p: &mut Parser) -> Parsed<String> {
    let mut name = identifier(p, "a superclass name after '<'")?;
    while *p.peek() == Token::Dot {
        if name.bytes().filter(|b| *b == b'.').count() >= 7 {
            return syntax(p, "A superclass name supports at most 8 parts");
        }
        p.take();
        let part = identifier(p, "a name after '.' in the superclass")?;
        name.push('.');
        name.push_str(&part);
    }
    Ok(name)
}

fn class_definition(p: &mut Parser) -> Parsed<ClassDefinition> {
    skip_separators(p);
    let start = byte(p);
    if !is_name(p, "classdef") {
        return syntax(p, "Expected 'classdef' at the start of a class file");
    }
    p.take();
    let mut attributes = Vec::new();
    for (name, value, at) in attribute_list(p)? {
        match name.as_str() {
            "Sealed" => {
                if flag(&name, &value, at)? {
                    attributes.push(name);
                }
            }
            _ => return unsupported(format!("the class attribute '{name}' is"), at),
        }
    }
    let name = identifier(p, "a class name after 'classdef'")?;
    let mut superclasses: Vec<(String, usize)> = Vec::new();
    if *p.peek() == Token::Less {
        p.take();
        loop {
            let at = byte(p);
            let base = superclass_name(p)?;
            if superclasses.iter().any(|(n, _)| *n == base) {
                p.error_byte = at;
                return Err(Failure::Syntax(format!(
                    "The superclass '{base}' is listed twice"
                )));
            }
            superclasses.push((base, at));
            if superclasses.len() > 16 {
                return syntax(p, "A class supports at most 16 direct superclasses");
            }
            if *p.peek() != Token::And {
                break;
            }
            p.take();
        }
    }
    let header_end = p.tokens[p.pos.saturating_sub(1)].end;
    if !matches!(
        p.peek(),
        Token::Newline | Token::Semi | Token::Comma | Token::Eof
    ) {
        return syntax(p, "Expected a line break after the classdef line");
    }
    let mut definition = ClassDefinition {
        name,
        attributes,
        superclasses,
        properties: Vec::new(),
        methods: Vec::new(),
        events: Vec::new(),
        enumeration: Vec::new(),
        functions: Vec::new(),
        span: start..header_end,
    };
    let mut members = BTreeSet::new();
    loop {
        skip_separators(p);
        let at = byte(p);
        let Token::Name(word) = p.peek().clone() else {
            if *p.peek() == Token::Eof {
                return syntax(p, "Expected 'end' to close the classdef block");
            }
            return syntax(
                p,
                "Expected properties, methods, events, enumeration or end in the classdef body",
            );
        };
        match word.as_str() {
            "end" => {
                p.take();
                break;
            }
            "properties" => {
                p.take();
                let block = property_block(p, at, &mut members)?;
                definition.properties.push(block);
            }
            "methods" => {
                p.take();
                let block = method_block(p, at, &mut members)?;
                definition.methods.push(block);
            }
            "events" => {
                p.take();
                events_block(p, &mut definition.events, &mut members)?;
            }
            "enumeration" => {
                p.take();
                enumeration_block(p, &mut definition.enumeration, &mut members)?;
            }
            _ => {
                return syntax(
                    p,
                    "Expected properties, methods, events, enumeration or end in the classdef body",
                );
            }
        }
        if definition.properties.len() + definition.methods.len() > 256 {
            return syntax(p, "A classdef file supports at most 256 blocks");
        }
    }
    // Local functions may follow the classdef block.
    loop {
        skip_separators(p);
        if *p.peek() == Token::Eof {
            break;
        }
        if !is_name(p, "function") {
            return syntax(
                p,
                "Only local function definitions may follow the classdef block",
            );
        }
        let start = byte(p);
        let function = function_decl::parse_function(p)?;
        let end = p.tokens[p.pos.saturating_sub(1)].end;
        if !members.insert(function.name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!(
                "Duplicate definition '{}' in the class file",
                function.name
            )));
        }
        definition.functions.push(Statement {
            kind: StatementKind::Function(Box::new(function)),
            span: start..end,
        });
        if definition.functions.len() > 128 {
            return syntax(p, "A class file supports at most 128 local functions");
        }
    }
    let endings: BTreeSet<_> = definition
        .functions
        .iter()
        .filter_map(|s| match &s.kind {
            StatementKind::Function(f) => Some(f.explicit_end),
            _ => None,
        })
        .collect();
    if endings.len() > 1 {
        return Err(Failure::Syntax(
            "Use end for every local function or omit it for every one".into(),
        ));
    }
    Ok(definition)
}
fn property_block(
    p: &mut Parser,
    at: usize,
    members: &mut BTreeSet<String>,
) -> Parsed<PropertyBlock> {
    let mut attributes = PropertyAttributes {
        get_access: Access::Public,
        set_access: Access::Public,
        constant: false,
        dependent: false,
        hidden: false,
        transient: false,
        immutable: false,
    };
    for (name, value, at) in attribute_list(p)? {
        match name.as_str() {
            "Access" => {
                let a = access(&name, &value, at)?;
                attributes.get_access = a;
                attributes.set_access = a;
                attributes.immutable = false;
            }
            "GetAccess" => attributes.get_access = access(&name, &value, at)?,
            "SetAccess" if value.as_deref() == Some("immutable") => {
                // Readable as GetAccess says; settable only by the constructor.
                attributes.set_access = Access::Private;
                attributes.immutable = true;
            }
            "SetAccess" => {
                attributes.set_access = access(&name, &value, at)?;
                attributes.immutable = false;
            }
            "Constant" => attributes.constant = flag(&name, &value, at)?,
            "Dependent" => attributes.dependent = flag(&name, &value, at)?,
            "Hidden" => attributes.hidden = flag(&name, &value, at)?,
            "Transient" => attributes.transient = flag(&name, &value, at)?,
            "SetObservable" | "GetObservable" => {
                if flag(&name, &value, at)? {
                    return unsupported("property listeners (SetObservable/GetObservable) are", at);
                }
            }
            _ => return unsupported(format!("the property attribute '{name}' is"), at),
        }
    }
    if attributes.constant && attributes.dependent {
        return Err(Failure::Syntax(
            "A property cannot be both Constant and Dependent".into(),
        ));
    }
    let mut properties = Vec::new();
    loop {
        skip_separators(p);
        let start = byte(p);
        if is_name(p, "end") {
            p.take();
            break;
        }
        if *p.peek() == Token::Eof {
            return syntax(p, "Expected 'end' to close the properties block");
        }
        let name = identifier(p, "a property name")?;
        let (dims, class_name, validators) =
            function_decl::parse_arg_constraints(p).map_err(|msg| {
                if p.error_byte == 0 {
                    p.error_byte = start;
                }
                Failure::Syntax(msg)
            })?;
        let validation =
            (dims.is_some() || class_name.is_some() || !validators.is_empty()).then(|| {
                ArgumentSpec {
                    group: None,
                    name: name.clone(),
                    dims,
                    class_name,
                    validators,
                    default: None,
                    byte: start,
                }
            });
        let default = match p.peek() {
            Token::Eq => {
                p.take();
                p.nodes = 0;
                Some(p.expr(0)?)
            }
            Token::Newline | Token::Semi | Token::Comma | Token::Eof => None,
            _ => {
                return syntax(
                    p,
                    "Expected a statement separator or '=' after property definition",
                );
            }
        };
        if !matches!(
            p.peek(),
            Token::Newline | Token::Semi | Token::Comma | Token::Eof
        ) {
            return syntax(
                p,
                "Expected a statement separator after property definition",
            );
        }
        let end = p.tokens[p.pos.saturating_sub(1)].end;
        if !members.insert(name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!(
                "Duplicate property or method name '{name}'"
            )));
        }
        if attributes.dependent && default.is_some() {
            p.error_byte = start;
            return Err(Failure::Syntax(format!(
                "The Dependent property '{name}' cannot have a default value"
            )));
        }
        properties.push(Property {
            name,
            validation,
            default,
            span: start..end,
        });
        if properties.len() > 1024 {
            return syntax(p, "A properties block supports at most 1024 properties");
        }
    }
    Ok(PropertyBlock {
        attributes,
        properties,
        byte: at,
    })
}

/// `events (Attributes) Name ... end`: the attributes (ListenAccess,
/// NotifyAccess, Hidden) are accepted and not enforced.
fn events_block(
    p: &mut Parser,
    events: &mut Vec<(String, usize)>,
    members: &mut BTreeSet<String>,
) -> Parsed<()> {
    for (name, _, at) in attribute_list(p)? {
        if !matches!(name.as_str(), "ListenAccess" | "NotifyAccess" | "Hidden") {
            return unsupported(format!("the event attribute '{name}' is"), at);
        }
    }
    loop {
        skip_separators(p);
        let start = byte(p);
        if is_name(p, "end") {
            p.take();
            return Ok(());
        }
        if *p.peek() == Token::Eof {
            return syntax(p, "Expected 'end' to close the events block");
        }
        let name = identifier(p, "an event name")?;
        if !members.insert(name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!("Duplicate member name '{name}'")));
        }
        events.push((name, start));
        if events.len() > 1024 {
            return syntax(p, "A class supports at most 1024 events");
        }
    }
}
/// `enumeration Name, Name(args) ... end`.
fn enumeration_block(
    p: &mut Parser,
    list: &mut Vec<EnumMember>,
    members: &mut BTreeSet<String>,
) -> Parsed<()> {
    if *p.peek() == Token::LParen {
        return syntax(p, "An enumeration block takes no attributes");
    }
    loop {
        skip_separators(p);
        let start = byte(p);
        if is_name(p, "end") {
            p.take();
            return Ok(());
        }
        if *p.peek() == Token::Eof {
            return syntax(p, "Expected 'end' to close the enumeration block");
        }
        let name = identifier(p, "an enumeration member name")?;
        let mut args = Vec::new();
        if *p.peek() == Token::LParen {
            p.take();
            if *p.peek() == Token::RParen {
                p.take();
            } else {
                loop {
                    p.nodes = 0;
                    args.push(p.expr(0)?);
                    match p.take() {
                        Token::Comma => {}
                        Token::RParen => break,
                        _ => return syntax(p, "Expected ',' or ')' after an enumeration argument"),
                    }
                }
            }
        }
        if !members.insert(name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!("Duplicate member name '{name}'")));
        }
        list.push(EnumMember {
            name,
            args,
            byte: start,
        });
        if list.len() > 1024 {
            return syntax(p, "A class supports at most 1024 enumeration members");
        }
    }
}

fn method_block(p: &mut Parser, at: usize, members: &mut BTreeSet<String>) -> Parsed<MethodBlock> {
    let mut attributes = MethodAttributes {
        is_static: false,
        access: Access::Public,
        is_abstract: false,
    };
    let mut has_attributes = false;
    for (name, value, at) in attribute_list(p)? {
        has_attributes = true;
        match name.as_str() {
            "Static" => attributes.is_static = flag(&name, &value, at)?,
            "Access" => attributes.access = access(&name, &value, at)?,
            "Abstract" => attributes.is_abstract = flag(&name, &value, at)?,
            _ => return unsupported(format!("the method attribute '{name}' is"), at),
        }
    }
    let mut methods = Vec::new();
    let mut signatures = Vec::new();
    if attributes.is_abstract {
        abstract_signatures(p, members, &mut signatures)?;
        return Ok(MethodBlock {
            attributes,
            methods,
            signatures,
            byte: at,
        });
    }
    loop {
        skip_separators(p);
        let start = byte(p);
        if is_name(p, "end") {
            p.take();
            break;
        }
        if !is_name(p, "function") {
            if *p.peek() == Token::Eof {
                return syntax(p, "Expected 'end' to close the methods block");
            }
            // `r = name(obj, x)` without a body: the method is in its own
            // file of the class folder. Whether the class has one is known
            // to the loader, not here.
            let name = signature_line(p, "Expected a function definition in the methods block")?;
            if !members.insert(name.clone()) {
                p.error_byte = start;
                return Err(Failure::Syntax(format!(
                    "Duplicate property or method name '{name}'"
                )));
            }
            signatures.push((name, start));
            if signatures.len() > 512 {
                return syntax(p, "A methods block supports at most 512 methods");
            }
            continue;
        }
        // `function v = get.Name(obj)` / `function obj = set.Name(obj, v)`:
        // the `get .` tokens are dropped for the signature parser and the
        // definition is renamed `get.Name` after it.
        let accessor = accessor_name(p);
        if accessor.is_some() && has_attributes {
            p.error_byte = start;
            return Err(Failure::Syntax(
                "Property get/set methods must be in a methods block without attributes".into(),
            ));
        }
        let mut function = function_decl::parse_function(p)?;
        if let Some(kind) = accessor {
            function.name = format!("{kind}.{}", function.name);
        }
        let end = p.tokens[p.pos.saturating_sub(1)].end;
        if !function.explicit_end {
            return unsupported("method declarations without a body are", start);
        }
        if !members.insert(function.name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!(
                "Duplicate property or method name '{}'",
                function.name
            )));
        }
        methods.push(Statement {
            kind: StatementKind::Function(Box::new(function)),
            span: start..end,
        });
        if methods.len() > 512 {
            return syntax(p, "A methods block supports at most 512 methods");
        }
    }
    Ok(MethodBlock {
        attributes,
        methods,
        signatures,
        byte: at,
    })
}

/// At `function`: for a `get.Name` / `set.Name` method, removes the `get`
/// and `.` tokens (the name follows) and returns `get` or `set`.
fn accessor_name(p: &mut Parser) -> Option<&'static str> {
    let line_end = p.tokens[p.pos..]
        .iter()
        .position(|t| matches!(t.token, Token::Newline | Token::Semi | Token::Eof))
        .map_or(p.tokens.len(), |n| p.pos + n);
    // The name is the first `get`/`set` directly followed by `.Name`, before
    // the input list.
    let at = (p.pos + 1..line_end.saturating_sub(2)).find(|&i| {
        matches!(&p.tokens[i].token, Token::Name(n) if n == "get" || n == "set")
            && p.tokens[i + 1].token == Token::Dot
            && matches!(p.tokens[i + 2].token, Token::Name(_))
            && p.tokens[i].end == p.tokens[i + 1].start
            && p.tokens[i + 1].end == p.tokens[i + 2].start
            && !p.tokens[p.pos..i].iter().any(|t| t.token == Token::LParen)
    })?;
    let kind = match &p.tokens[at].token {
        Token::Name(n) if n == "get" => "get",
        _ => "set",
    };
    p.tokens.drain(at..at + 2);
    Some(kind)
}

/// One method signature without a body: `r = name(args)`,
/// `[a, b] = name(args)`, `name(args)` or `name`. Returns the name;
/// `expected` is the error for any other line.
fn signature_line(p: &mut Parser, expected: &str) -> Parsed<String> {
    let mut depth = 0usize;
    let line: Vec<Token> = p.tokens[p.pos..]
        .iter()
        .map(|t| t.token.clone())
        .take_while(|t| {
            match t {
                Token::LParen | Token::LBracket => depth += 1,
                Token::RParen | Token::RBracket => depth = depth.saturating_sub(1),
                _ => {}
            }
            !matches!(t, Token::Newline | Token::Semi | Token::Eof)
                && !(depth == 0 && *t == Token::Comma)
        })
        .collect();
    let after_eq = line
        .iter()
        .position(|t| *t == Token::Eq)
        .map_or(0, |i| i + 1);
    let Some(Token::Name(name)) = line.get(after_eq) else {
        return syntax(p, expected);
    };
    let name = name.clone();
    if !crate::files::valid_name(&name) || keyword(&name) {
        return syntax(p, expected);
    }
    let rest = &line[after_eq + 1..];
    let balanced = rest.is_empty()
        || (rest.first() == Some(&Token::LParen) && rest.last() == Some(&Token::RParen));
    if !balanced {
        return syntax(p, expected);
    }
    for _ in 0..line.len() {
        p.take();
    }
    Ok(name)
}

/// The lines of a `methods (Abstract)` block: signatures without bodies,
/// optionally after `function`.
fn abstract_signatures(
    p: &mut Parser,
    members: &mut BTreeSet<String>,
    signatures: &mut Vec<(String, usize)>,
) -> Parsed<()> {
    loop {
        skip_separators(p);
        let start = byte(p);
        if is_name(p, "end") {
            p.take();
            return Ok(());
        }
        if *p.peek() == Token::Eof {
            return syntax(p, "Expected 'end' to close the methods block");
        }
        if is_name(p, "function") {
            p.take();
        }
        let name = signature_line(
            p,
            "Expected an abstract method signature such as 'r = name(obj)'",
        )?;
        if !members.insert(name.clone()) {
            p.error_byte = start;
            return Err(Failure::Syntax(format!(
                "Duplicate property or method name '{name}'"
            )));
        }
        signatures.push((name, start));
        if signatures.len() > 512 {
            return syntax(p, "A methods block supports at most 512 methods");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_class_files_after_comments() {
        assert!(is_class_file("classdef Pt\nend"));
        assert!(is_class_file(
            "% help\n\n  %more\r\nclassdef (Sealed) Pt\nend"
        ));
        assert!(!is_class_file("classdefx = 1"));
        assert!(!is_class_file("x = 1;\nclassdef Pt\nend"));
        assert!(!is_class_file("function y = f\nend"));
    }

    #[test]
    fn parses_blocks_attributes_and_local_functions() {
        let c = parse_class_file(
            "classdef Pt\n properties\n  x = 1\n  y\n end\n properties (Constant)\n  C = 2;\n end\n methods\n  function obj = Pt(x)\n   obj.x = x;\n  end\n end\n methods (Static, Access = private)\n  function r = make()\n   r = 1;\n  end\n end\nend\nfunction h = helper\nh = 1;\nend\n",
        )
        .unwrap();
        assert_eq!(c.name, "Pt");
        assert_eq!(c.properties.len(), 2);
        assert!(c.properties[1].attributes.constant);
        assert_eq!(c.methods[1].attributes.access, Access::Private);
        assert!(c.methods[1].attributes.is_static);
        assert_eq!(c.functions.len(), 1);
    }

    #[test]
    fn parses_superclasses_accessors_and_abstract_signatures() {
        let c = parse_class_file(
            "classdef (Sealed) C < A & B & handle\n properties (Dependent, Hidden)\n  d\n end\n properties (SetAccess = immutable, Transient)\n  k = 1\n end\n methods\n  function v = get.d(obj)\n   v = obj.k;\n  end\n  function obj = set.d(obj, v)\n   obj.k = v;\n  end\n end\n methods (Abstract)\n  r = area(obj)\n  [a, b] = two(obj, x)\n  function bare\n  plain(obj)\n end\nend\n",
        )
        .unwrap();
        let names: Vec<_> = c.superclasses.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["A", "B", "handle"]);
        assert_eq!(c.attributes, ["Sealed"]);
        assert!(c.properties[0].attributes.dependent && c.properties[0].attributes.hidden);
        assert!(c.properties[1].attributes.immutable && c.properties[1].attributes.transient);
        assert_eq!(c.properties[1].attributes.get_access, Access::Public);
        let methods: Vec<_> = c.methods[0]
            .methods
            .iter()
            .map(|s| match &s.kind {
                StatementKind::Function(f) => f.name.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(methods, ["get.d", "set.d"]);
        assert!(c.methods[1].attributes.is_abstract);
        let abstract_names: Vec<_> = c.methods[1]
            .signatures
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(abstract_names, ["area", "two", "bare", "plain"]);
        assert!(parse_class_file("classdef C < A & A\nend").is_err());
        assert!(parse_class_file("classdef C < pkg.A & pkg.A\nend").is_err());
        assert!(
            parse_class_file("classdef C\n properties (Dependent)\n  d = 1\n end\nend").is_err()
        );
    }

    #[test]
    fn parses_method_signatures_without_a_body() {
        let c = parse_class_file(
            "classdef Pt\n methods\n  r = norm2(obj)\n  function obj = Pt(x)\n  end\n  [a, b] = two(obj, x)\n end\n methods (Static, Access = private)\n  p = make(x)\n  bare\n end\nend\n",
        )
        .unwrap();
        let names = |block: &MethodBlock| -> Vec<String> {
            block.signatures.iter().map(|(n, _)| n.clone()).collect()
        };
        assert_eq!(names(&c.methods[0]), ["norm2", "two"]);
        assert_eq!(c.methods[0].methods.len(), 1);
        assert!(!c.methods[0].attributes.is_abstract);
        assert_eq!(names(&c.methods[1]), ["make", "bare"]);
        assert!(c.methods[1].attributes.is_static);
        assert_eq!(c.methods[1].attributes.access, Access::Private);
        // Not a signature: the old message; a name used twice is rejected.
        for (source, message) in [
            (
                "classdef Pt\n methods\n  x = 1 + 2\n end\nend",
                "Expected a function definition in the methods block",
            ),
            (
                "classdef Pt\n methods\n  v = get.x(obj)\n end\nend",
                "Expected a function definition in the methods block",
            ),
            (
                "classdef Pt\n methods\n  r = m(obj)\n  function r = m(obj)\n  end\n end\nend",
                "Duplicate property or method name 'm'",
            ),
        ] {
            let Err(ClassFileError::Syntax(d)) = parse_class_file(source) else {
                panic!("{source}");
            };
            assert!(d.to_string().contains(message), "{source}: {d}");
        }
    }

    #[test]
    fn parses_package_qualified_superclasses() {
        let c = parse_class_file(
            "classdef (Sealed) B < pkg.A & handle & a.b.A\n methods\n  function obj = B(x)\n   obj = obj@pkg.A(x);\n   m@a.b.A(obj);\n  end\n end\nend\n",
        )
        .unwrap();
        assert_eq!(c.name, "B");
        assert_eq!(c.attributes, ["Sealed"]);
        let names: Vec<_> = c.superclasses.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["pkg.A", "handle", "a.b.A"]);
        let body = format!("{:?}", c.methods);
        assert!(body.contains("obj@pkg.A"), "{body}");
        assert!(body.contains("m@a.b.A"), "{body}");
        let err = parse_class_file("classdef B < a.b.c.d.e.f.g.h.I\nend").unwrap_err();
        assert!(
            err.message("B.m", "classdef B < a.b.c.d.e.f.g.h.I\nend")
                .contains("at most 8 parts"),
            "{err:?}"
        );
    }

    #[test]
    fn unsupported_features_name_their_line() {
        // Property validation `x (1,1) double` is supported since fix 18.
        for (source, feature, line) in [
            (
                "classdef (Abstract) Pt\nend",
                "class attribute 'Abstract'",
                1,
            ),
            (
                "classdef Pt\n properties (SetObservable)\n  x\n end\nend",
                "property listeners",
                2,
            ),
        ] {
            let Err(ClassFileError::Unsupported { feature: f, byte }) = parse_class_file(source)
            else {
                panic!("{source}");
            };
            assert!(f.contains(feature), "{source}: {f}");
            assert_eq!(
                Diagnostic::new(source, byte, String::new()).line,
                line,
                "{source}"
            );
        }
    }
}
