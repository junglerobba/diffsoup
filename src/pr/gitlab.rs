use crate::{
    error::{CustomError, Result},
    pr::{HistoryEntry, Page, PageDirection, Pagination, PrFetcher},
};
use error_stack::ResultExt;
use jj_lib::backend::CommitId;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use url::Url;

#[derive(Debug)]
pub struct GitlabFetcher {
    client: reqwest::blocking::Client,
    host: String,
    project: String,
    repository: String,
    mr_id: String,
}

impl GitlabFetcher {
    pub fn new(url: &Url, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &token {
            headers.insert(
                "PRIVATE-TOKEN",
                token.parse().change_context(CustomError::UrlError)?,
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
            [project, repository, _, "merge_requests", mr_id] => Ok(Self {
                client,
                host: host.to_string(),
                project: project.to_string(),
                repository: repository.to_string(),
                mr_id: mr_id.to_string(),
            }),
            _ => Err(CustomError::UrlError.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct MergeRequestVersion {
    head_commit_sha: String,
    start_commit_sha: String,
}

impl TryFrom<Vec<MergeRequestVersion>> for Page<HistoryEntry> {
    type Error = error_stack::Report<CustomError>;

    fn try_from(versions: Vec<MergeRequestVersion>) -> std::result::Result<Self, Self::Error> {
        let items = versions
            .iter()
            .map(|v| {
                match (
                    CommitId::try_from_hex(&v.head_commit_sha),
                    CommitId::try_from_hex(&v.start_commit_sha),
                ) {
                    (Some(head), Some(start)) => Ok(HistoryEntry {
                        head_ref: head,
                        base_ref: Some(start),
                    }),
                    (Some(head), _) => Ok(HistoryEntry {
                        head_ref: head,
                        base_ref: None,
                    }),
                    (None, _) => Err(CustomError::CommitError(format!(
                        "invalid commit hash {} from gitlab",
                        v.start_commit_sha
                    ))),
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Self {
            items,
            next: None,
            direction: PageDirection::Backward,
        })
    }
}

impl PrFetcher for GitlabFetcher {
    fn fetch_history(&self, _pagination: Option<&Pagination>) -> Result<Page<HistoryEntry>> {
        let res: Vec<MergeRequestVersion> = self
            .client
            .get(format!(
                "{}/api/v4/projects/{}%2F{}/merge_requests/{}/versions",
                self.host, self.project, self.repository, self.mr_id
            ))
            .send()
            .change_context(CustomError::RequestError)?
            .json()
            .change_context(CustomError::RequestError)?;
        res.try_into()
    }
}
