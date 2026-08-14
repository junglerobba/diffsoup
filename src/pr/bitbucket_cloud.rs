use std::{cell::RefCell, collections::HashMap};

use error_stack::ResultExt;
use jj_lib::backend::CommitId;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::Deserialize;
use url::Url;

use crate::{
    error::{CustomError, Result, SendChecked},
    pr::{CursorPagination, HistoryEntry, Page, PageDirection, Pagination, PrFetcher},
};

#[derive(Debug)]
pub struct BitbucketCloudFetcher {
    client: reqwest::blocking::Client,
    workspace: String,
    repo: String,
    pr_id: String,
    resolved_commits: RefCell<HashMap<String, CommitId>>,
}

impl BitbucketCloudFetcher {
    pub fn new(url: &Url, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &token {
            headers.insert(
                AUTHORIZATION,
                format!("Basic {}", token)
                    .parse()
                    .change_context(CustomError::RequestError)?,
            );
        }
        let client = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .build()
            .change_context(CustomError::ProcessError(
                "error building client".to_string(),
            ))?;
        let segments: Vec<&str> = url
            .path_segments()
            .ok_or_else(|| CustomError::UrlError(format!("Invalid URL format {url}")))?
            .collect();

        match segments.as_slice() {
            [workspace, repo, "pull-requests", pr_id, ..] => Ok(Self {
                client,
                workspace: workspace.to_string(),
                repo: repo.to_string(),
                pr_id: pr_id.to_string(),
                resolved_commits: RefCell::new(HashMap::new()),
            }),
            _ => Err(CustomError::UrlError(format!(
                "Url {url} does not math expected format for bitbucket cloud"
            ))
            .into()),
        }
    }

    fn resolve_commit_id(&self, hash: impl AsRef<str>) -> Result<CommitId> {
        if let Some(commit) = self.resolved_commits.borrow().get(hash.as_ref()) {
            return Ok(commit.clone());
        };

        let res: Commit = self
            .client
            .get(format!(
                "https://api.bitbucket.org/2.0/repositories/{}/{}/commit/{}",
                self.workspace,
                self.repo,
                hash.as_ref()
            ))
            .send()
            .change_context(CustomError::RequestError)?
            .json()
            .change_context(CustomError::RequestError)?;

        let commit_id = CommitId::try_from_hex(&res.hash).ok_or(CustomError::CommitError(
            "invalid commit hash from bitbucket".into(),
        ))?;
        let mut commits = self.resolved_commits.borrow_mut();
        commits.insert(hash.as_ref().to_string(), commit_id.clone());

        Ok(commit_id)
    }

    fn map_history(&self, value: PrActivity) -> Result<Page<HistoryEntry>> {
        let actions = value.values.iter().filter_map(|v| v.update.as_ref());

        let mut commits = Vec::new();

        for action in actions.rev() {
            let head_ref = self.resolve_commit_id(&action.source.commit.hash)?;
            let base_ref = Some(self.resolve_commit_id(&action.destination.commit.hash)?);
            commits.push(HistoryEntry::new(head_ref, base_ref));
        }

        Ok(Page {
            items: commits,
            next: value.next.map(|url| {
                Pagination::Cursor(CursorPagination {
                    cursor: Some(url),
                    limit: 0,
                    direction: PageDirection::Backward,
                })
            }),
            direction: PageDirection::Backward,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PrActivity {
    next: Option<String>,
    values: Vec<PrActivityEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PrActivityEntry {
    #[serde(default)]
    update: Option<UpdateAction>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateAction {
    destination: PrRef,
    source: PrRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PrRef {
    commit: Commit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Commit {
    hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PullRequest {
    source: PrEndpoint,
    destination: PrEndpoint,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct PrEndpoint {
    branch: Branch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Branch {
    name: String,
}

const BASE_PATH: &str = "https://api.bitbucket.org/2.0";

impl PrFetcher for BitbucketCloudFetcher {
    fn get_pull_request(&self) -> Result<Option<super::PullRequest>> {
        let res: PullRequest = self
            .client
            .get(format!(
                "{BASE_PATH}/repositories/{}/{}/pullrequests/{}",
                self.workspace, self.repo, self.pr_id
            ))
            .send_checked()?
            .json()
            .change_context(CustomError::RequestError)?;
        Ok(Some(super::PullRequest {
            target_branch: res.destination.branch.name,
            source_branch: res.source.branch.name,
        }))
    }

    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<HistoryEntry>> {
        let next = match pagination {
            None => Some(format!(
                "{BASE_PATH}/repositories/{}/{}/pullrequests/{}/activity",
                self.workspace, self.repo, self.pr_id,
            )),
            Some(Pagination::Cursor(pagination)) => pagination.cursor.clone(),
            _ => {
                return Err(CustomError::ProcessError(
                    "cursor based pagination is required for bitbucket".to_string(),
                )
                .into());
            }
        }
        .ok_or(CustomError::RequestError)?;
        let res: PrActivity = self
            .client
            .get(next)
            .send_checked()?
            .json()
            .change_context(CustomError::RequestError)?;
        self.map_history(res)
    }
}
