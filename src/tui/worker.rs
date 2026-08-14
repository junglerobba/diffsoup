use std::{
    sync::{Arc, mpsc::Receiver},
    thread::JoinHandle,
};

use diffsoup::{
    diff::{CommitDiff, calculate_branch_diff, get_commit},
    error::{CustomError, Result},
    pr::{HistoryEntry, Page, Pagination, PrFetcher},
    repo::{ensure_commits_exist, fetch_commits},
    trees::DiffTree,
};
use error_stack::ResultExt;
use jj_lib::{
    backend::CommitId,
    ref_name::RefName,
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
        init: bool,
        pagination: Option<Pagination>,
    },
    CalculateBranchDiff {
        from: CommitId,
        from_index: usize,
        to: CommitId,
        to_index: usize,
        target: Option<CommitId>,
    },
    RenderInterdiff {
        from: Option<CommitId>,
        to: Option<CommitId>,
        render_width: u16,
        scroll: u16,
    },
}

#[derive(Debug, Clone)]
pub enum WorkerResponse {
    Error(String),
    Loading(String),
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
        page: Page<HistoryEntry>,
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
                WorkerRequest::LoadCommits { init, pagination } => {
                    let local = if init && let Ok(Some(pr_meta)) = pr_fetcher.get_pull_request() {
                        (
                            repo.view()
                                .get_local_bookmark(RefName::new(&pr_meta.source_branch))
                                .as_normal(),
                            repo.view()
                                .get_local_bookmark(RefName::new(&pr_meta.target_branch))
                                .as_normal(),
                        )
                    } else {
                        (None, None)
                    };

                    match pr_fetcher.fetch_history(pagination.as_ref()) {
                        Ok(mut page) => {
                            if let (Some(source), target) = local
                                && page.latest().map(|e| &e.head_ref) != Some(source)
                            {
                                page.insert(
                                    HistoryEntry::new(source.clone(), target.cloned())
                                        .pending(true),
                                );
                            }
                            let items = page
                                .items
                                .iter()
                                .flat_map(|entry| [entry.base_ref.as_ref(), Some(&entry.head_ref)])
                                .flatten();
                            let missing = ensure_commits_exist(items, repo.as_ref())?;
                            if !missing.is_empty() {
                                worker_response_tx
                                    .send(WorkerMsg {
                                        job_id: request.job_id,
                                        msg: WorkerResponse::Loading(format!(
                                            "Missing {} commits, fetching from remote...",
                                            missing.len()
                                        )),
                                    })
                                    .change_context(CustomError::ProcessError(
                                        "worker: error sending response".to_string(),
                                    ))?;
                                repo = fetch_commits(&missing, repo)?;
                            };
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
                    target,
                } => calculate_branch_diff(&from, &to, target.as_ref(), &workspace, repo.as_ref())
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

fn render_interdiff(
    from_sha: &Option<CommitId>,
    to_sha: &Option<CommitId>,
    workspace: &Workspace,
    repo: &impl Repo,
    render_width: u16,
    scroll: u16,
) -> WorkerResponse {
    let from_commit = from_sha
        .as_ref()
        .map(|sha| get_commit(sha, repo))
        .transpose()
        .unwrap_or(None);
    let to_commit = to_sha
        .as_ref()
        .map(|sha| get_commit(sha, repo))
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
