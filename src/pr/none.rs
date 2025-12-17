use jj_lib::ref_name::RefNameBuf;

use crate::pr::{Page, PageDirection, Pagination, PrFetcher};

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
    ) -> crate::error::Result<Page<RefNameBuf>> {
        let commits = vec![RefNameBuf::from(&self.from), RefNameBuf::from(&self.to)];
        Ok(Page {
            items: commits,
            direction: PageDirection::Backward,
            next: None,
        })
    }
}
