use std::{
    sync::{Arc, mpsc::Receiver},
    thread::JoinHandle,
};

use diffsoup::{
    diff::{CommitDiff, calculate_branch_diff, get_commit},
    error::{CustomError, Result},
    pr::{Page, Pagination, PrFetcher},
    repo::ensure_commits_exist,
    trees::DiffTree,
};
use error_stack::ResultExt;
use jj_lib::{
    ref_name::RefNameBuf,
    repo::{ReadonlyRepo, Repo},
    workspace::Workspace,
};

use crate::tui::{JobId, WorkerSender};

#[derive(Debug, Clone)]
pub struct WorkerMsg<T> {
    pub job_id: JobId,
    pub msg: T,
}

#[derive(Debug, Clone)]
pub enum WorkerRequest {
    LoadCommits {
        pagination: Option<Pagination>,
    },
    CalculateBranchDiff {
        from: String,
        from_index: usize,
        to: String,
        to_index: usize,
    },
    RenderInterdiff {
        from: Option<String>,
        to: Option<String>,
        render_width: u16,
        scroll: u16,
    },
}

#[derive(Debug, Clone)]
pub enum WorkerResponse {
    Error(String),
    CalculateBranchDiff {
        commits: Vec<CommitDiff>,
        from: usize,
        to: usize,
    },
    RenderInterdiff {
        title: String,
        diff: String,
        scroll: u16,
    },
    LoadCommits {
        page: Page<RefNameBuf>,
    },
}

pub fn spawn_worker_thread(
    worker_response_tx: WorkerSender,
    worker_request_rx: Receiver<WorkerMsg<WorkerRequest>>,
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    pr_fetcher: Box<dyn PrFetcher>,
) -> JoinHandle<Result<()>> {
    let mut repo = repo;
    std::thread::spawn(move || {
        while let Ok(request) = worker_request_rx.recv() {
            let response = match request.msg {
                WorkerRequest::LoadCommits { pagination } => {
                    match pr_fetcher.fetch_history(pagination.as_ref()) {
                        Ok(page) => {
                            repo = ensure_commits_exist(page.items.iter(), repo)?;
                            WorkerResponse::LoadCommits { page }
                        }
                        Err(e) => WorkerResponse::Error(format!("{:#?}", e)),
                    }
                }
                WorkerRequest::CalculateBranchDiff {
                    from,
                    from_index,
                    to,
                    to_index,
                } => calculate_branch_diff(&from, &to, &workspace, repo.as_ref())
                    .map(|diff| WorkerResponse::CalculateBranchDiff {
                        commits: diff,
                        from: from_index,
                        to: to_index,
                    })
                    .unwrap_or_else(|e| WorkerResponse::Error(format!("{:#?}", e))),
                WorkerRequest::RenderInterdiff {
                    from,
                    to,
                    render_width,
                    scroll,
                } => render_interdiff(&from, &to, &workspace, repo.as_ref(), render_width, scroll),
            };
            worker_response_tx
                .send(WorkerMsg {
                    job_id: request.job_id,
                    msg: response,
                })
                .change_context(CustomError::ProcessError(
                    "worker: error sending response".to_string(),
                ))?;
        }
        Ok(())
    })
}

pub fn render_interdiff(
    from_sha: &Option<String>,
    to_sha: &Option<String>,
    workspace: &Workspace,
    repo: &impl Repo,
    render_width: u16,
    scroll: u16,
) -> WorkerResponse {
    let from_commit = from_sha
        .as_ref()
        .map(|sha| get_commit(sha, workspace, repo))
        .transpose()
        .unwrap_or(None);
    let to_commit = to_sha
        .as_ref()
        .map(|sha| get_commit(sha, workspace, repo))
        .transpose()
        .unwrap_or(None);

    let trees = DiffTree::from(from_commit.as_ref(), to_commit.as_ref());

    trees
        .map(|tree| {
            diffsoup::diff::render_interdiff(&tree, workspace, repo, render_width)
                .map(|diff| WorkerResponse::RenderInterdiff {
                    title: format!("{tree}"),
                    diff,
                    scroll,
                })
                .unwrap_or_else(|e| WorkerResponse::Error(format!("{:#?}", e)))
        })
        .unwrap_or(WorkerResponse::Error(
            "no commits in this diff to render".to_string(),
        ))
}
