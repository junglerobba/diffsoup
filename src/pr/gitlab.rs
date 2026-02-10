use error_stack::ResultExt;
use jj_lib::backend::CommitId;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::Deserialize;
use url::Url;

use crate::{
    error::{CustomError, Result},
    pr::{Page, PageDirection, Pagination, PrFetcher},
};

#[derive(Debug)]
pub struct GitlabFetcher {
    client: reqwest::blocking::Client,
    owner: String,
    repo: String,
    pr_id: u64,
}

impl GitlabFetcher {
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
        let segments: Vec<&str> = url.path_segments().ok_or(CustomError::UrlError)?.collect();

        match segments.as_slice() {
            [owner, repo, "-", "merge_requests", pr_id, ..] => Ok(Self {
                client,
                owner: owner.to_string(),
                repo: repo.to_string(),
                pr_id: pr_id.parse().change_context(CustomError::UrlError)?,
            }),
            [owner, repo, "merge_requests", pr_id, ..] => Ok(Self {
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
struct Version {
    head_commit_sha: String,
}

impl PrFetcher for GitlabFetcher {
    fn fetch_history(&self, _: Option<&Pagination>) -> Result<Page<CommitId>> {
        let res = self
            .client
            .get(format!(
                "https://gitlab.com/api/v4/projects/{}%2F{}/merge_requests/{}/versions",
                self.owner, self.repo, self.pr_id
            ))
            .send()
            .change_context(CustomError::RequestError)?;
        let res: Vec<Version> = res.json().change_context(CustomError::RequestError)?;
        Ok(Page {
            items: res
                .into_iter()
                .rev()
                .map(|x| CommitId::try_from_hex(&x.head_commit_sha).unwrap())
                .collect(),
            next: None,
            direction: PageDirection::Backward,
        })
    }
}
