use jj_lib::backend::CommitId;

use crate::{
    error::CustomError,
    pr::{Page, PageDirection, Pagination, PrFetcher},
};

#[derive(Debug)]
pub struct NoFetcher {
    from: String,
    to: String,
}

impl NoFetcher {
    pub fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

impl PrFetcher for NoFetcher {
    fn fetch_history(
        &self,
        _pagination: Option<&Pagination>,
    ) -> crate::error::Result<Page<CommitId>> {
        let commits = vec![
            CommitId::try_from_hex(&self.from).ok_or(CustomError::CommitError(
                "must provide a valid commit hash".into(),
            ))?,
            CommitId::try_from_hex(&self.to).ok_or(CustomError::CommitError(
                "must provide a valid commit hash".into(),
            ))?,
        ];
        Ok(Page {
            items: commits,
            direction: PageDirection::Backward,
            next: None,
        })
    }
}
