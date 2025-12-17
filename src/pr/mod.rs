mod bitbucket;
mod github;
mod none;

use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;
use std::fmt::Debug;

use crate::{
    error::{CustomError, Result},
    pr::{bitbucket::BitbucketFetcher, github::GithubFetcher, none::NoFetcher},
};

#[derive(Debug, Clone, Copy, Default)]
pub enum PageDirection {
    #[default]
    Forward,
    Backward,
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub direction: PageDirection,
    pub next: Option<Pagination>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OffsetPagination {
    offset: usize,
    limit: Option<usize>,
    direction: PageDirection,
}

#[derive(Debug, Clone, Default)]
pub struct CursorPagination {
    cursor: Option<String>,
    limit: usize,
    direction: PageDirection,
}

#[derive(Debug, Clone)]
pub enum Pagination {
    Offset(OffsetPagination),
    Cursor(CursorPagination),
}

impl Pagination {
    pub fn direction(&self) -> PageDirection {
        match self {
            Pagination::Offset(offset) => offset.direction,
            Pagination::Cursor(cursor) => cursor.direction,
        }
    }
}

pub trait PrFetcher: Debug + Send {
    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<RefNameBuf>>;
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

            if host.contains("github.com") {
                let token = std::env::var("GITHUB_TOKEN").ok();
                Ok(Some(Box::new(GithubFetcher::new(&parsed, token)?)))
            } else if host.contains("bitbucket") {
                let token = std::env::var("BITBUCKET_TOKEN").ok();
                Ok(Some(Box::new(BitbucketFetcher::new(&parsed, token)?)))
            } else {
                Ok(None)
            }
        }
        (_, _, _) => Ok(None),
    }
}
