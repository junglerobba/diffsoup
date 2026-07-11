use error_stack::ResultExt;
use jj_lib::backend::CommitId;
use reqwest::header::{AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde_json::json;
use std::{io, process::Stdio};
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

pub(crate) fn get_token() -> Result<Option<String>> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .stdout(Stdio::piped())
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).change_context(CustomError::ProcessError("Error running gh".into()));
        }
    };

    if !output.status.success() {
        return Err(CustomError::ProcessError(format!(
            "`gh` exited with status {:?}",
            output.status.code().unwrap_or(-1)
        ))
        .into());
    }
    let token = String::from_utf8(output.stdout).change_context(CustomError::ProcessError(
        "invalid gh auth token output".into(),
    ))?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.into()))
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

impl TryFrom<GraphQlResponse> for Page<CommitId> {
    type Error = error_stack::Report<CustomError>;

    fn try_from(value: GraphQlResponse) -> Result<Self> {
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
                commits.push(CommitId::try_from_hex(&entry.node.before_commit.oid).ok_or(
                    CustomError::CommitError("invalid commit hash from github".into()),
                )?);
            }
            commits.push(CommitId::try_from_hex(&entry.node.after_commit.oid).ok_or(
                CustomError::CommitError("invalid commit hash from github".into()),
            )?);
        }

        Ok(Self {
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
        })
    }
}

impl PrFetcher for GithubFetcher {
    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<CommitId>> {
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
        res.try_into()
    }
}
