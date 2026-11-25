use super::*;
use std::collections::BTreeSet;

fn name(p: &mut Parser) -> Result<String> {
    match p.take() {
        Token::Name(n) if crate::files::valid_name(&n) && !keyword(&n) => Ok(n),
        _ => Err("Expected an ASCII function/argument name of at most 63 characters".into()),
    }
}

pub(super) fn parse_function(p: &mut Parser) -> Result<FunctionDefinition> {
    p.take(); // function
    let mut outputs = Vec::new();
    if *p.peek() == Token::LBracket {
        p.take();
        while *p.peek() != Token::RBracket {
            outputs.push(name(p)?);
            if outputs.len() > 32 {
                return Err("A function supports at most 32 outputs".into());
            }
            if *p.peek() == Token::Comma {
                p.take();
                if *p.peek() == Token::RBracket {
                    return Err("Expected an output name after comma".into());
                }
            } else if *p.peek() != Token::RBracket
                && (!matches!(p.peek(), Token::Name(_))
                    || p.tokens[p.pos].start == p.tokens[p.pos - 1].end)
            {
                return Err("Expected a comma or space between output names".into());
            }
        }
        p.take();
        p.expect(Token::Eq)?;
    } else if p
        .tokens
        .get(p.pos + 1)
        .is_some_and(|t| t.token == Token::Eq)
    {
        outputs.push(name(p)?);
        p.take();
    }
    let function_name = name(p)?;
    let mut inputs = Vec::new();
    if *p.peek() == Token::LParen {
        p.take();
        while *p.peek() != Token::RParen {
            inputs.push(if *p.peek() == Token::Ignore {
                p.take();
                None
            } else {
                Some(name(p)?)
            });
            if inputs.len() > 32 {
                return Err("A function supports at most 32 inputs".into());
            }
            if *p.peek() == Token::RParen {
                break;
            }
            p.expect(Token::Comma)?;
            if *p.peek() == Token::RParen {
                return Err("Expected an input name after comma".into());
            }
        }
        p.expect(Token::RParen)?;
    }
    if inputs
        .iter()
        .take(inputs.len().saturating_sub(1))
        .flatten()
        .any(|n| n == "varargin")
        || outputs
            .iter()
            .take(outputs.len().saturating_sub(1))
            .any(|n| n == "varargout")
    {
        return Err("Variable argument lists must be last.".into());
    }
    if inputs.iter().flatten().collect::<BTreeSet<_>>().len() != inputs.iter().flatten().count()
        || outputs.iter().collect::<BTreeSet<_>>().len() != outputs.len()
    {
        return Err("Duplicate function input or output name".into());
    }
    separator(p, true)?;
    let arguments = parse_arguments_blocks(p, &inputs, &outputs)?;
    let body = parse_block(p, 0, false)?;
    let explicit_end = matches!(p.peek(), Token::Name(n) if n == "end");
    if explicit_end {
        end_block(p)?;
    }
    Ok(FunctionDefinition {
        name: function_name,
        inputs,
        outputs,
        arguments,
        body,
        explicit_end,
    })
}

/// An open definition is ambiguous until we see a function-closing end.
/// If there are none, all headers introduce file-local functions. Otherwise
/// each end closes the innermost definition and resumes its parent's body.
pub(super) fn parse_file(p: &mut Parser, function_file: bool) -> Result<Vec<Statement>> {
    let mut roots = if function_file {
        Vec::new()
    } else {
        parse_block(p, 0, false)?
    };
    let mut open: Vec<(std::ops::Range<usize>, FunctionDefinition)> = Vec::new();
    let mut any_end = false;
    let mut count = 0;
    loop {
        while is_separator(p.peek()) {
            p.take();
        }
        let start = p.tokens[p.pos].start;
        p.error_byte = start;
        let (start, finished) = match p.peek() {
            Token::Eof => break,
            Token::Name(n) if n == "function" => {
                count += 1;
                if count > 128 {
                    return Err("A source file supports at most 128 functions".into());
                }
                let definition = parse_function(p)?;
                if !definition.explicit_end {
                    open.push((start..p.tokens[p.pos - 1].end, definition));
                    continue;
                }
                (start, definition)
            }
            Token::Name(n) if n == "end" && !open.is_empty() => {
                end_block(p)?;
                let (span, mut parent) = open.pop().unwrap();
                parent.explicit_end = true;
                (span.start, parent)
            }
            Token::Name(n) if ends_block(n) => {
                return Err(format!("Unexpected block keyword {:?}", p.peek()));
            }
            _ => {
                let body = if let Some((_, parent)) = open.last_mut() {
                    &mut parent.body
                } else if !function_file {
                    &mut roots
                } else {
                    return Err("Function files may contain only function definitions".into());
                };
                body.extend(parse_block(p, 0, false)?);
                continue;
            }
        };
        any_end = true;
        let siblings = open.last_mut().map_or(&mut roots, |(_, f)| &mut f.body);
        if siblings.iter().any(|s| {
            matches!(&s.kind,
            StatementKind::Function(f) if f.name == finished.name)
        }) {
            return Err(format!("Duplicate function definition '{}'", finished.name));
        }
        siblings.push(Statement {
            kind: StatementKind::Function(Box::new(finished)),
            span: start..p.tokens[p.pos - 1].end,
        });
    }
    if !open.is_empty() && !function_file {
        return Err("Local functions in scripts require an explicit end".into());
    }
    if any_end && !open.is_empty() {
        return Err(
            "Use end for every function or omit it for every function in a function file".into(),
        );
    }
    // In an implicit-end file, each body ends at the next function header.
    roots.extend(open.into_iter().map(|(span, definition)| Statement {
        kind: StatementKind::Function(Box::new(definition)),
        span,
    }));
    Ok(roots)
}

pub(super) type ArgConstraints = (Option<Vec<ArgDim>>, Option<String>, Vec<ArgValidator>);

pub(super) fn parse_arg_constraints(p: &mut Parser) -> Result<ArgConstraints> {
    let dims = if *p.peek() == Token::LParen {
        p.take();
        if *p.peek() == Token::RParen {
            p.error_byte = p.tokens[p.pos].start;
            return Err("Expected dimension size or ':' in arguments block.".into());
        }
        let mut d = Vec::new();
        loop {
            match *p.peek() {
                Token::Colon => {
                    p.take();
                    d.push(ArgDim::Colon);
                }
                Token::Number(v) => {
                    p.take();
                    if !v.is_finite() || v < 0.0 || v.fract() != 0.0 || v > (usize::MAX as f64) {
                        return Err(
                            "Dimensions in an arguments block must be nonnegative integers or ':'."
                                .into(),
                        );
                    }
                    d.push(ArgDim::Fixed(v as usize));
                }
                _ => {
                    p.error_byte = p.tokens[p.pos].start;
                    return Err("Expected dimension size or ':' in arguments block.".into());
                }
            }
            if *p.peek() == Token::Comma {
                p.take();
            } else {
                break;
            }
        }
        p.expect(Token::RParen)?;
        Some(d)
    } else {
        None
    };
    let class_name = if let Token::Name(mut cls) = p.peek().clone() {
        p.take();
        while *p.peek() == Token::Dot {
            p.take();
            let sub = name(p)?;
            cls.push('.');
            cls.push_str(&sub);
        }
        Some(cls)
    } else {
        None
    };
    let validators = if *p.peek() == Token::LBrace {
        p.take();
        let mut vs = Vec::new();
        while *p.peek() != Token::RBrace {
            while *p.peek() == Token::Newline {
                p.take();
            }
            if *p.peek() == Token::RBrace {
                break;
            }
            let v_byte = p.tokens[p.pos].start;
            let v_name = name(p)?;
            let v_args = if *p.peek() == Token::LParen {
                p.take();
                let mut a = Vec::new();
                while *p.peek() != Token::RParen {
                    a.push(p.expr(0)?);
                    if *p.peek() == Token::Comma {
                        p.take();
                    } else if *p.peek() != Token::RParen {
                        p.error_byte = p.tokens[p.pos].start;
                        return Err("Expected ',' or ')' in validator call.".into());
                    }
                }
                p.expect(Token::RParen)?;
                a
            } else {
                Vec::new()
            };
            vs.push(ArgValidator {
                name: v_name,
                args: v_args,
                byte: v_byte,
            });
            while *p.peek() == Token::Newline {
                p.take();
            }
            if matches!(p.peek(), Token::Comma | Token::Semi) {
                p.take();
            } else if *p.peek() != Token::RBrace {
                p.error_byte = p.tokens[p.pos].start;
                return Err("Expected ',' or '}' after validator in arguments block.".into());
            }
        }
        p.expect(Token::RBrace)?;
        vs
    } else {
        Vec::new()
    };
    Ok((dims, class_name, validators))
}

fn parse_arguments_blocks(
    p: &mut Parser,
    inputs: &[Option<String>],
    outputs: &[String],
) -> Result<Vec<ArgumentsBlock>> {
    let mut blocks = Vec::new();
    let mut seen_repeating = false;
    let mut seen_name_value = false;
    let mut seen_nv_names = BTreeSet::new();
    let mut positional_offset = 0usize;
    loop {
        while matches!(p.peek(), Token::Newline | Token::Semi | Token::Comma) {
            p.take();
        }
        let Token::Name(n) = p.peek() else {
            break;
        };
        if n != "arguments" {
            break;
        }
        let block_byte = p.tokens[p.pos].start;
        let next = p.tokens.get(p.pos + 1).map(|t| &t.token);
        let arguments_is_param = inputs.iter().flatten().any(|inp| inp == "arguments")
            || outputs.iter().any(|out| out == "arguments");
        let kind = if !arguments_is_param
            && matches!(next, Some(Token::Newline | Token::Semi | Token::Comma))
        {
            p.take();
            separator(p, false)?;
            ArgumentsKind::Input
        } else if matches!(next, Some(Token::LParen))
            && let Some(Token::Name(k)) = p.tokens.get(p.pos + 2).map(|t| &t.token)
            && matches!(k.as_str(), "Input" | "Output" | "Repeating")
            && matches!(
                p.tokens.get(p.pos + 3).map(|t| &t.token),
                Some(Token::RParen)
            )
            && matches!(
                p.tokens.get(p.pos + 4).map(|t| &t.token),
                Some(Token::Newline | Token::Semi | Token::Comma)
            )
        {
            let attr = k.clone();
            p.take(); // arguments
            p.take(); // (
            p.take(); // Input | Output | Repeating
            p.take(); // )
            separator(p, false)?;
            match attr.as_str() {
                "Output" => ArgumentsKind::Output,
                "Repeating" => {
                    if seen_repeating {
                        return Err(
                            "A function can have at most one arguments (Repeating) block.".into(),
                        );
                    }
                    seen_repeating = true;
                    ArgumentsKind::Repeating
                }
                _ => ArgumentsKind::Input,
            }
        } else {
            break;
        };

        let mut specs = Vec::new();
        loop {
            while matches!(p.peek(), Token::Newline | Token::Semi | Token::Comma) {
                p.take();
            }
            if *p.peek() == Token::Eof {
                p.error_byte = p.tokens[p.pos].start;
                return Err("Expected 'end' to close arguments block".into());
            }
            if matches!(p.peek(), Token::Name(n) if n == "end") {
                p.take();
                separator(p, true)?;
                break;
            }
            let spec_byte = p.tokens[p.pos].start;
            let first_name = name(p)?;
            let (group, arg_name) = if *p.peek() == Token::Dot {
                p.take();
                let field_name = name(p)?;
                (Some(first_name), field_name)
            } else {
                (None, first_name)
            };
            if group.is_some() && kind != ArgumentsKind::Input {
                p.error_byte = spec_byte;
                return Err(
                    "Name-value arguments are only allowed in input arguments blocks.".into(),
                );
            }
            if group.is_some() {
                seen_name_value = true;
                if !seen_nv_names.insert(arg_name.to_ascii_lowercase()) {
                    p.error_byte = spec_byte;
                    return Err(format!("Duplicate name-value argument '{arg_name}'."));
                }
            } else if kind == ArgumentsKind::Input && seen_name_value {
                p.error_byte = spec_byte;
                return Err("Positional arguments cannot follow name-value arguments.".into());
            }
            let (dims, class_name, validators) = parse_arg_constraints(p)?;
            let default = if *p.peek() == Token::Eq {
                p.take();
                Some(p.expr(0)?)
            } else {
                None
            };
            if kind == ArgumentsKind::Repeating && default.is_some() {
                p.error_byte = spec_byte;
                return Err("Repeating arguments cannot have default values.".into());
            }
            if kind == ArgumentsKind::Output && default.is_some() {
                p.error_byte = spec_byte;
                return Err("Output arguments cannot have default values.".into());
            }
            separator(p, false)?;
            specs.push(ArgumentSpec {
                group,
                name: arg_name,
                dims,
                class_name,
                validators,
                default,
                byte: spec_byte,
            });
        }
        match kind {
            ArgumentsKind::Input => {
                let positional: Vec<&ArgumentSpec> =
                    specs.iter().filter(|s| s.group.is_none()).collect();
                let mut groups: Vec<&str> = Vec::new();
                for spec in &specs {
                    if let Some(g) = &spec.group
                        && !groups.contains(&g.as_str())
                    {
                        groups.push(g.as_str());
                    }
                }
                if positional_offset + positional.len() + groups.len() > inputs.len() {
                    return Err(
                        "Arguments block declares more inputs than the function signature.".into(),
                    );
                }
                for (i, spec) in positional.iter().enumerate() {
                    let sig_idx = positional_offset + i;
                    if let Some(Some(inp)) = inputs.get(sig_idx)
                        && spec.name != *inp
                    {
                        return Err(format!(
                            "Argument '{}' in arguments block must match function input '{}' at position {}.",
                            spec.name,
                            inp,
                            sig_idx + 1
                        ));
                    }
                }
                for (i, group_name) in groups.iter().enumerate() {
                    let sig_idx = positional_offset + positional.len() + i;
                    if let Some(Some(inp)) = inputs.get(sig_idx)
                        && *group_name != inp
                    {
                        return Err(format!(
                            "Argument '{}' in arguments block must match function input '{}' at position {}.",
                            group_name,
                            inp,
                            sig_idx + 1
                        ));
                    }
                }
                positional_offset += positional.len() + groups.len();
            }
            ArgumentsKind::Output => {
                if specs.len() > outputs.len() {
                    return Err("Arguments (Output) block declares more outputs than the function signature.".into());
                }
                for (i, spec) in specs.iter().enumerate() {
                    if let Some(out_name) = outputs.get(i)
                        && spec.name != *out_name
                    {
                        return Err(format!(
                            "Output argument '{}' in arguments block must match function output '{}' at position {}.",
                            spec.name,
                            out_name,
                            i + 1
                        ));
                    }
                }
            }
            ArgumentsKind::Repeating => {
                if positional_offset + specs.len() > inputs.len() {
                    return Err("Arguments (Repeating) block declares more inputs than the function signature.".into());
                }
                for (i, spec) in specs.iter().enumerate() {
                    let sig_idx = positional_offset + i;
                    if let Some(Some(inp)) = inputs.get(sig_idx)
                        && spec.name != *inp
                    {
                        return Err(format!(
                            "Argument '{}' in arguments block must match function input '{}' at position {}.",
                            spec.name,
                            inp,
                            sig_idx + 1
                        ));
                    }
                }
                positional_offset += specs.len();
            }
        }
        blocks.push(ArgumentsBlock {
            kind,
            specs,
            byte: block_byte,
        });
    }
    Ok(blocks)
}

pub(super) fn validate_program(statements: &[Statement]) -> Result<()> {
    let function_file = statements
        .first()
        .is_some_and(|s| matches!(s.kind, StatementKind::Function(_)));
    let mut names = BTreeSet::new();
    let mut endings = BTreeSet::new();
    for statement in statements {
        if let StatementKind::Function(f) = &statement.kind {
            if !names.insert(&f.name) {
                return Err(format!("Duplicate function definition '{}'", f.name));
            }
            if names.len() > 128 {
                return Err("A source file supports at most 128 functions".into());
            }
            endings.insert(f.explicit_end);
            if !function_file && !f.explicit_end {
                return Err("Local functions in scripts require an explicit end".into());
            }
        } else if function_file {
            return Err("Function files may contain only function definitions".into());
        }
    }
    if endings.len() > 1 {
        return Err(
            "Use end for every function or omit it for every function in a function file".into(),
        );
    }
    Ok(())
}
