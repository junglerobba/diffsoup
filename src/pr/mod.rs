mod bitbucket;

use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;

use crate::{
    error::{CustomError, Result},
    pr::bitbucket::BitbucketFetcher,
};

pub trait PrFetcher {
    fn fetch_history(&self) -> Result<PrHistory>;
}

pub fn get_pr_fetcher(url: &str) -> Result<Option<Box<dyn PrFetcher>>> {
    let parsed = url::Url::parse(url).change_context(CustomError::UrlError)?;
    let host = parsed.host_str().ok_or(CustomError::UrlError)?;

    if host.contains("bitbucket") {
        let token = std::env::var("BITBUCKET_TOKEN").ok();
        Ok(Some(Box::new(BitbucketFetcher::new(&parsed, token)?)))
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
pub struct PrHistory(pub Vec<RefNameBuf>);
