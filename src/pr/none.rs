use jj_lib::ref_name::RefNameBuf;

use crate::pr::{PrFetcher, PrHistory};

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
        _offset: usize,
        _limit: Option<usize>,
    ) -> crate::error::Result<PrHistory> {
        let commits = vec![RefNameBuf::from(&self.from), RefNameBuf::from(&self.to)];
        Ok(PrHistory {
            commits,
            offset: 0,
            limit: None,
            last_page: true,
        })
    }
}
