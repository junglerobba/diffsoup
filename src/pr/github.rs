use error_stack::ResultExt;
use gix::ThreadSafeRepository;
use jj_lib::backend::CommitId;
use reqwest::header::{AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::{
    error::{CustomError, Result},
    pr::{HistoryEntry, Page, PageDirection, Pagination, PrFetcher},
};

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
const DEFAULT_PAGE_SIZE: usize = 25;

#[derive(Debug)]
pub struct GithubFetcher {
    client: reqwest::blocking::Client,
    owner: String,
    repo_name: String,
    pr_id: usize,
    repo: ThreadSafeRepository,
}

impl GithubFetcher {
    pub fn new(url: &Url, token: Option<String>, repo: gix::Repository) -> Result<Self> {
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
            [owner, repo_name, "pull", pr_id, ..] => Ok(Self {
                client,
                owner: owner.to_string(),
                repo_name: repo_name.to_string(),
                pr_id: pr_id.parse().change_context(CustomError::UrlError)?,
                repo: repo.into_sync(),
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
    node: TimelineEvent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadRefForcePushedEvent {
    before_commit: Commit,
    after_commit: Commit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseRefChangedEvent {
    current_ref_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[serde(tag = "__typename")]
pub enum TimelineEvent {
    HeadRefForcePushedEvent(HeadRefForcePushedEvent),
    BaseRefChangedEvent(BaseRefChangedEvent),
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

impl TryFrom<(&ThreadSafeRepository, GraphQlResponse)> for Page<HistoryEntry> {
    type Error = error_stack::Report<CustomError>;

    fn try_from(value: (&ThreadSafeRepository, GraphQlResponse)) -> Result<Self> {
        let (repo, response) = value;
        let page_info = response
            .data
            .repository
            .pull_request
            .timeline_items
            .page_info;
        let mut commits = Vec::new();
        let mut base_ref = None;
        for (i, entry) in response
            .data
            .repository
            .pull_request
            .timeline_items
            .edges
            .iter()
            .enumerate()
        {
            match &entry.node {
                TimelineEvent::HeadRefForcePushedEvent(event) => {
                    if !page_info.has_previous_page && i == 0 {
                        commits.push(HistoryEntry {
                            head_ref: CommitId::try_from_hex(&event.before_commit.oid).ok_or(
                                CustomError::CommitError("invalid commit hash from github".into()),
                            )?,
                            base_ref: base_ref.clone(),
                        });
                    }
                    commits.push(HistoryEntry {
                        head_ref: CommitId::try_from_hex(&event.after_commit.oid).ok_or(
                            CustomError::CommitError("invalid commit hash from github".into()),
                        )?,
                        base_ref: base_ref.clone(),
                    });
                }
                TimelineEvent::BaseRefChangedEvent(event) => {
                    let repo = repo.to_thread_local();
                    let Some(id) = repo
                        .try_find_reference(&event.current_ref_name)
                        .change_context(CustomError::RepoError)?
                        .and_then(|reference| reference.try_id())
                    else {
                        continue;
                    };
                    let Some(commit_id) = CommitId::try_from_hex(id.to_hex().to_string()) else {
                        continue;
                    };
                    base_ref = Some(commit_id);
                }
            }
        }

        Ok(Self {
            items: commits,
            next: page_info.has_previous_page.then_some(Pagination::Cursor(
                super::CursorPagination {
                    cursor: page_info.start_cursor,
                    limit: response
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
    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<HistoryEntry>> {
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
                "repo": self.repo_name,
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
        (&self.repo, res).try_into()
    }
}
