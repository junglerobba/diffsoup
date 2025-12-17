mod bitbucket;
mod none;

use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;
use std::fmt::Debug;

use crate::{
    error::{CustomError, Result},
    pr::bitbucket::BitbucketFetcher,
    pr::none::NoFetcher,
};

pub trait PrFetcher: Debug + Send {
    fn fetch_history(&self, offset: usize, limit: Option<usize>) -> Result<PrHistory>;
}

pub fn get_pr_fetcher(
    url: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<Option<Box<dyn PrFetcher>>> {
    match (url, from, to) {
        (None, Some(from), Some(to)) => Ok(Some(Box::new(NoFetcher::new(&from, &to)))),
        (Some(url), _, _) => {
            let parsed = url::Url::parse(&url).change_context(CustomError::UrlError)?;
            let host = parsed.host_str().ok_or(CustomError::UrlError)?;

            if host.contains("bitbucket") {
                let token = std::env::var("BITBUCKET_TOKEN").ok();
                Ok(Some(Box::new(BitbucketFetcher::new(&parsed, token)?)))
            } else {
                Ok(None)
            }
        }
        (_, _, _) => Ok(None),
    }
}

#[derive(Debug)]
pub struct PrHistory {
    pub commits: Vec<RefNameBuf>,
    pub offset: usize,
    pub limit: Option<usize>,
    pub last_page: bool,
}
