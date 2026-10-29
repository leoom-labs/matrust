use super::variable_edit_job::Token;
use matrust::{
    Array, Engine, Result,
    variable_edit::cell_replace::{self, Binding, CellTarget},
};

pub struct Request {
    pub revision: Token,
    pub editor: Token,
    pub target: CellTarget,
    pub source: Binding,
}
pub struct Finished {
    pub editor: Token,
    pub target: CellTarget,
    pub result: Result<Array>,
}
pub fn perform(engine: &mut Engine, revision: &Token, request: Request) -> Finished {
    let result = if revision.same(&request.revision) {
        cell_replace::replace(engine, &request.target, &request.source)
    } else {
        Err("Workspace operation completed; select the replacement source and target again".into())
    };
    Finished {
        editor: request.editor,
        target: request.target,
        result,
    }
}
