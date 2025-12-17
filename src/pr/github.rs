use error_stack::ResultExt;
use jj_lib::ref_name::RefNameBuf;
use reqwest::header::{AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::{
    error::{CustomError, Result},
    pr::{Page, PageDirection, Pagination, PrFetcher},
};

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
const DEFAULT_PAGE_SIZE: usize = 25;

#[derive(Debug)]
pub struct GithubFetcher {
    client: reqwest::blocking::Client,
    owner: String,
    repo: String,
    pr_id: usize,
}

impl GithubFetcher {
    pub fn new(url: &Url, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            "graphql-client"
                .parse()
                .change_context(CustomError::UrlError)?,
        );
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
        let segments: Vec<&str> = url.path_segments().ok_or(CustomError::UrlError)?.collect();

        match segments.as_slice() {
            [owner, repo, "pull", pr_id, ..] => Ok(Self {
                client,
                owner: owner.to_string(),
                repo: repo.to_string(),
                pr_id: pr_id.parse().change_context(CustomError::UrlError)?,
            }),
            _ => Err(CustomError::UrlError.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GraphQlResponse {
    data: Data,
}

#[derive(Debug, Deserialize)]
pub struct Data {
    repository: Repository,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pull_request: PullRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    timeline_items: TimelineItems,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItems {
    edges: Vec<Edge>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
pub struct Edge {
    node: Node,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    before_commit: Commit,
    after_commit: Commit,
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    has_previous_page: bool,
    start_cursor: Option<String>,
}

impl From<GraphQlResponse> for Page<RefNameBuf> {
    fn from(value: GraphQlResponse) -> Self {
        let page_info = value.data.repository.pull_request.timeline_items.page_info;
        let mut commits = Vec::new();
        for (i, entry) in value
            .data
            .repository
            .pull_request
            .timeline_items
            .edges
            .iter()
            .enumerate()
        {
            if !page_info.has_previous_page && i == 0 {
                commits.push(RefNameBuf::from(&entry.node.before_commit.oid));
            }
            commits.push(RefNameBuf::from(&entry.node.after_commit.oid));
        }

        Self {
            items: commits,
            next: page_info.has_previous_page.then_some(Pagination::Cursor(
                super::CursorPagination {
                    cursor: page_info.start_cursor,
                    limit: value
                        .data
                        .repository
                        .pull_request
                        .timeline_items
                        .edges
                        .len(),
                    direction: PageDirection::Backward,
                },
            )),
            direction: PageDirection::Backward,
        }
    }
}

impl PrFetcher for GithubFetcher {
    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<RefNameBuf>> {
        let (cursor, limit) = match pagination {
            None => (None.as_ref(), DEFAULT_PAGE_SIZE),
            Some(Pagination::Cursor(pagination)) => (pagination.cursor.as_ref(), pagination.limit),
            _ => {
                return Err(CustomError::ProcessError(
                    "cursor based pagination is required for github".to_string(),
                )
                .into());
            }
        };
        let query = include_str!("github_query.graphql");
        let body = json!({
            "query" : query,
            "variables": {
                "owner": self.owner,
                "repo": self.repo,
                "pr": self.pr_id,
                "cursor": cursor,
                "limit": limit
            }
        });
        let res = self
            .client
            .post(GITHUB_GRAPHQL_URL)
            .json(&body)
            .send()
            .change_context(CustomError::RequestError)?;
        let res: GraphQlResponse = res.json().change_context(CustomError::RequestError)?;
        Ok(res.into())
    }
}
