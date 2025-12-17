use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::Deserialize;
use url::Url;

use crate::{
    error::{CustomError, Result},
    pr::{OffsetPagination, Page, PageDirection, Pagination, PrFetcher},
};

#[derive(Debug)]
pub struct BitbucketFetcher {
    client: reqwest::blocking::Client,
    host: String,
    project: String,
    repo: String,
    pr_id: String,
}

impl BitbucketFetcher {
    pub fn new(url: &Url, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &token {
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", token)
                    .parse()
                    .change_context(CustomError::UrlError)?,
            );
        }
        let client = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .build()
            .change_context(CustomError::ProcessError(
                "error building client".to_string(),
            ))?;
        let host = url.origin().unicode_serialization();
        let segments: Vec<&str> = url.path_segments().ok_or(CustomError::UrlError)?.collect();

        match segments.as_slice() {
            [
                "projects",
                project,
                "repos",
                repo,
                "pull-requests",
                pr_id,
                ..,
            ] => Ok(Self {
                client,
                host: host.to_string(),
                project: project.to_string(),
                repo: repo.to_string(),
                pr_id: pr_id.to_string(),
            }),
            _ => Err(CustomError::UrlError.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrActivity {
    is_last_page: bool,
    limit: Option<usize>,
    _next_page_start: Option<u32>,
    _size: usize,
    start: u32,
    values: Vec<PrActivityEntry>,
}

impl From<PrActivity> for Page<RefNameBuf> {
    fn from(value: PrActivity) -> Self {
        let actions = value.values.iter().filter_map(|v| match v {
            PrActivityEntry::Rescoped(action) => Some(action),
            _ => None,
        });

        let mut commits = Vec::new();

        for (i, action) in actions.rev().enumerate() {
            if value.is_last_page && i == 0 {
                commits.push(RefNameBuf::from(&action.previous_from_hash));
            }
            commits.push(RefNameBuf::from(&action.from_hash));
        }

        Self {
            items: commits,
            next: (!value.is_last_page).then_some(Pagination::Offset(OffsetPagination {
                offset: (value.start as usize) + value.limit.unwrap_or(value.values.len()),
                limit: value.limit,
                direction: PageDirection::Backward,
            })),
            direction: PageDirection::Backward,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "UPPERCASE")]
enum PrActivityEntry {
    Rescoped(PrRescopeAction),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrRescopeAction {
    from_hash: String,
    previous_from_hash: String,
    _to_hash: String,
    _previous_to_hash: String,
}

impl PrFetcher for BitbucketFetcher {
    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<RefNameBuf>> {
        let (offset, limit) = match pagination {
            None => (0, None),
            Some(Pagination::Offset(pagination)) => (pagination.offset, pagination.limit),
            _ => {
                return Err(CustomError::ProcessError(
                    "offset based pagination is required for bitbucket".to_string(),
                )
                .into());
            }
        };
        let res: PrActivity = self
            .client
            .get(format!(
                "{}/rest/api/latest/projects/{}/repos/{}/pull-requests/{}/activities?start={}{}",
                self.host,
                self.project,
                self.repo,
                self.pr_id,
                offset,
                limit
                    .map(|limit| format!("&limit={limit}"))
                    .unwrap_or_default()
            ))
            .send()
            .change_context(CustomError::RequestError)?
            .json()
            .change_context(CustomError::RequestError)?;
        Ok(res.into())
    }
}
