use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;

use crate::error::{CustomError, Result};

pub trait PrFetcher {
    fn fetch_history(&self) -> Result<PrHistory>;
}

pub fn get_pr_fetcher(url: &str) -> Result<Option<Box<dyn PrFetcher>>> {
    let _parsed = url::Url::parse(url).change_context(CustomError::UrlError)?;

    Ok(None)
}

#[derive(Debug)]
pub struct PrHistory(pub Vec<RefNameBuf>);
