use std::sync::Arc;

use error_stack::ResultExt;
use futures::executor::block_on_stream;
use jj_cli::{cli_util::load_revset_aliases, ui::Ui};
use jj_lib::{
    backend::CommitId,
    repo::{ReadonlyRepo, Repo},
    workspace::Workspace,
};

use crate::{
    diff::parse_revset_expr,
    error::{CustomError, Result},
    pr::{HistoryEntry, Page, PageDirection, Pagination, PrFetcher},
};

#[derive(Debug)]
pub struct NoFetcher {
    from: CommitId,
    to: CommitId,
}

fn load_commit(id: &str, repo: &impl Repo, workspace: &Workspace) -> Result<CommitId> {
    let aliases_map = load_revset_aliases(&Ui::null(), workspace.settings().config())
        .map_err(|_| CustomError::RepoError)?;
    let (id, _) = block_on_stream(
        parse_revset_expr(id, workspace, repo, &aliases_map)?
            .evaluate(repo)
            .change_context(CustomError::ExprError)?
            .commit_change_ids(),
    )
    .next()
    .ok_or(CustomError::ExprError)
    .attach_opaque_with(|| format!("could not resolve expression {} to a commit", id))?
    .change_context(CustomError::RepoError)?;
    Ok(id)
}

impl NoFetcher {
    pub fn new(
        from: &str,
        to: &str,
        repo: Arc<ReadonlyRepo>,
        workspace: &Workspace,
    ) -> Result<Self> {
        let from = load_commit(from, repo.as_ref(), workspace)?;
        let to = load_commit(to, repo.as_ref(), workspace)?;
        Ok(Self { from, to })
    }
}

impl PrFetcher for NoFetcher {
    fn fetch_history(
        &self,
        _pagination: Option<&Pagination>,
    ) -> crate::error::Result<Page<HistoryEntry>> {
        let commits = vec![self.from.clone().into(), self.to.clone().into()];
        Ok(Page {
            items: commits,
            direction: PageDirection::Backward,
            next: None,
        })
    }
}
