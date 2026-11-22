use error_stack::ResultExt;
use jj_lib::backend::CommitId;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::{Deserialize, Deserializer};
use std::cell::OnceCell;
use url::Url;

use crate::{
    error::{CustomError, Result, SendChecked},
    pr::{HistoryEntry, OffsetPagination, Page, PageDirection, Pagination, PrFetcher, PullRequest},
};

#[derive(Debug)]
pub struct ForgejoFetcher {
    client: reqwest::blocking::Client,
    pr_url: String,
    host: String,
    repo_owner: String,
    repo_name: String,
    pr_id: String,
    meta: OnceCell<PullRequestMeta>,
}

#[derive(Debug, Deserialize)]
struct PullRequestMeta {
    base: PrBranchInfo,
    head: PrBranchInfo,
}

#[derive(Debug, Deserialize)]
struct PrBranchInfo {
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct IssueTimelineItem {
    #[serde(flatten)]
    action: IssueTimelineAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum IssueTimelineAction {
    PullPush {
        #[serde(deserialize_with = "from_json_string")]
        body: PullPushEvent,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct PullPushEvent {
    commit_ids: Vec<String>,
}

fn from_json_string<'de, D, T>(deserializer: D) -> core::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let s = String::deserialize(deserializer)?;
    serde_json::from_str(&s).map_err(serde::de::Error::custom)
}

impl ForgejoFetcher {
    pub fn new(url: &Url, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &token {
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", token)
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
        let host = url.origin().unicode_serialization();
        let segments: Vec<&str> = url
            .path_segments()
            .ok_or_else(|| CustomError::UrlError(format!("Invalid URL format {url}")))?
            .collect();

        match segments.as_slice() {
            [repo_owner, repo_name, "pulls", pr_id, ..] => Ok(Self {
                client,
                pr_url: url.to_string(),
                host: host.to_string(),
                repo_owner: repo_owner.to_string(),
                repo_name: repo_name.to_string(),
                pr_id: pr_id.to_string(),
                meta: OnceCell::new(),
            }),
            _ => Err(CustomError::UrlError(format!(
                "Invalid URL format on {url} for platform forgejo"
            ))
            .into()),
        }
    }

    fn get_meta(&self) -> Result<&PullRequestMeta> {
        if let Some(value) = self.meta.get() {
            return Ok(value);
        }
        let res: PullRequestMeta = self
            .client
            .get(format!(
                "{}/api/v1/repos/{}/{}/pulls/{}",
                self.host, self.repo_owner, self.repo_name, self.pr_id
            ))
            .send_checked()?
            .json()
            .change_context(CustomError::RequestError)?;
        let _ = self.meta.set(res);
        Ok(self.meta.get().unwrap())
    }
}

impl PrFetcher for ForgejoFetcher {
    fn pr_url(&self) -> Option<String> {
        Some(self.pr_url.clone())
    }

    fn get_pull_request(&self) -> Result<Option<super::PullRequest>> {
        let meta = self.get_meta()?;
        Ok(Some(PullRequest {
            target_branch: meta.base.ref_name.clone(),
            source_branch: meta.head.ref_name.clone(),
        }))
    }

    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<HistoryEntry>> {
        const LIMIT: usize = 30;
        let (page, limit) = match pagination {
            None => (1, Some(LIMIT)),
            Some(Pagination::Offset(pagination)) => (pagination.offset, pagination.limit),
            _ => {
                return Err(CustomError::ProcessError(
                    "offset based pagination is required for bitbucket".to_string(),
                )
                .into());
            }
        };
        let base = self.get_meta().and_then(|meta| {
            CommitId::try_from_hex(&meta.base.sha).ok_or_else(|| {
                CustomError::CommitError(format!("invalid commit hash {}", meta.base.sha)).into()
            })
        })?;
        let res: Option<Vec<IssueTimelineItem>> = self
            .client
            .get(format!(
                "{}/api/v1/repos/{}/{}/issues/{}/timeline?page={}{}",
                self.host,
                self.repo_owner,
                self.repo_name,
                self.pr_id,
                page,
                limit
                    .map(|limit| format!("&limit={limit}"))
                    .unwrap_or_default()
            ))
            .send_checked()?
            .json()
            .change_context(CustomError::RequestError)?;

        let Some(res) = res else {
            return Ok(Page {
                items: Vec::new(),
                direction: PageDirection::Forward,
                next: None,
            });
        };

        let next = (!res.is_empty()).then_some(Pagination::Offset(OffsetPagination {
            direction: PageDirection::Forward,
            limit: Some(LIMIT),
            offset: page + 1,
        }));

        let commits = res
            .into_iter()
            .filter_map(|i| match i.action {
                IssueTimelineAction::PullPush { body } => body.commit_ids.into_iter().last(),
                IssueTimelineAction::Unknown => None,
            })
            .map(|commit_id| {
                let commit_id = CommitId::try_from_hex(&commit_id).ok_or_else(|| {
                    CustomError::CommitError(format!("invalid commit hash {commit_id}"))
                })?;
                Ok(HistoryEntry::new(commit_id, Some(base.clone())))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Page {
            items: commits,
            direction: PageDirection::Forward,
            next,
        })
    }
}
