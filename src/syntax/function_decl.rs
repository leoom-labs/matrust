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
    let body = parse_block(p, 0, false, true)?;
    let explicit_end = matches!(p.peek(), Token::Name(n) if n == "end");
    if explicit_end {
        end_block(p)?;
    }
    Ok(FunctionDefinition {
        name: function_name,
        inputs,
        outputs,
        body,
        explicit_end,
    })
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
        return Err("Use end for every function or omit it for every function in a function file; nested functions are not supported yet".into());
    }
    Ok(())
}
