//! Revision-guarded requests executed by the existing workspace worker.
use matrust::{
    Engine, Result,
    variable_edit::{self, Rect, Target},
};
use std::sync::Arc;

/// Process-local generation; clones share only a token, never a payload.
#[derive(Clone, Default, Debug)]
pub struct Token(Arc<()>);
impl Token {
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
pub enum Action {
    Literal(String),
    Paste(String),
    Copy,
    CopyText,
}
pub struct Request {
    pub revision: Token,
    pub editor: Token,
    pub target: Target,
    pub rectangle: Rect,
    pub action: Action,
}
pub enum Effect {
    Edited(variable_edit::Committed),
    Copied(String),
}
pub struct Finished {
    pub editor: Token,
    pub target: Target,
    pub result: Result<Effect>,
}
pub fn perform(engine: &mut Engine, revision: &Token, request: Request) -> Finished {
    let result = if !revision.same(&request.revision) {
        Err("Workspace operation completed; reopen or refresh the variable editor before applying or copying".into())
    } else {
        match request.action {
            Action::Literal(text) => variable_edit::apply_literal(
                engine,
                &request.target,
                request.rectangle.row(),
                request.rectangle.col(),
                &text,
            )
            .map(Effect::Edited),
            Action::Paste(text) => {
                variable_edit::paste_tsv(engine, &request.target, request.rectangle, &text)
                    .map(Effect::Edited)
            }
            Action::CopyText => {
                variable_edit::copy_text_row(engine, &request.target).map(Effect::Copied)
            }
            Action::Copy => variable_edit::copy_tsv(engine, &request.target, request.rectangle)
                .map(Effect::Copied),
        }
    };
    Finished {
        editor: request.editor,
        target: request.target,
        result,
    }
}
