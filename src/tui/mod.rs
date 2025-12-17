use std::sync::{
    Arc,
    mpsc::{self, SendError, Sender},
};

use diffsoup::pr::PrFetcher;
use jj_lib::{repo::ReadonlyRepo, workspace::Workspace};

use crate::tui::{
    app::spawn_ui_thread,
    state::{AppScreen, AppState, UiEvent},
    worker::{WorkerMsg, WorkerRequest, WorkerResponse, spawn_worker_thread},
};

mod app;
mod state;
mod worker;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct JobId(u64);

impl JobId {
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

#[derive(Debug)]
pub enum MainThreadMsg {
    Worker(WorkerMsg<WorkerResponse>),
    Ui(UiEvent),
}

#[derive(Debug, Clone)]
pub struct WorkerSender(Sender<MainThreadMsg>);
impl WorkerSender {
    fn send(&self, msg: WorkerMsg<WorkerResponse>) -> Result<(), SendError<MainThreadMsg>> {
        self.0.send(MainThreadMsg::Worker(msg))
    }
}

#[derive(Debug, Clone)]
pub struct UiSender(Sender<MainThreadMsg>);
impl UiSender {
    fn send(&self, msg: UiEvent) -> Result<(), SendError<MainThreadMsg>> {
        self.0.send(MainThreadMsg::Ui(msg))
    }
}

pub fn run(
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    pr_fetcher: Box<dyn PrFetcher>,
) -> anyhow::Result<()> {
    let (view_tx, view_rx) = mpsc::channel();
    let (worker_request_tx, worker_request_rx) = mpsc::channel();
    let (main_tx, main_rx) = mpsc::channel();

    let mut app = AppState::new(worker_request_tx);

    let ui_handle = spawn_ui_thread(UiSender(main_tx.clone()), view_rx);
    let worker_handle = spawn_worker_thread(
        WorkerSender(main_tx),
        worker_request_rx,
        workspace,
        repo,
        pr_fetcher,
    );

    let job_id = app.next_job();
    // init by loading commits
    let _ = app.worker_req_tx.send(WorkerMsg {
        job_id,
        msg: WorkerRequest::LoadCommits {
            offset: 0,
            limit: None,
        },
    });
    app.current_job = Some(job_id);

    let mut exit = false;
    while !exit {
        match main_rx.recv()? {
            MainThreadMsg::Worker(response)
                if app.current_job.is_some_and(|id| id == response.job_id) =>
            {
                app.handle_worker(response.msg);
            }
            // discard if event is outdated
            MainThreadMsg::Worker(_) => {}
            MainThreadMsg::Ui(action) => match action {
                UiEvent::SizeChange(size) => {
                    app.screen_size = size;
                }
                UiEvent::Exit => {
                    app.screen = state::AppScreen::Exit;
                    exit = true;
                }
                UiEvent::Scroll(event) => match &mut app.screen {
                    AppScreen::List(list_view) => {
                        let current = list_view.list_state.selected().unwrap_or_default();
                        let new = event.get_new_index(
                            app.screen_size,
                            current,
                            list_view.get_visible_commits().len(),
                        );
                        app.list_state.select(Some(new));
                        list_view.list_state.select(Some(new));
                    }
                    AppScreen::DiffView(diff_view) => {
                        diff_view.scroll = event
                            .get_new_index(
                                app.screen_size,
                                diff_view.scroll.into(),
                                diff_view.diff.lines().count(),
                            )
                            .try_into()
                            .unwrap_or_default();
                    }
                    _ => {}
                },
                UiEvent::PatchsetChange((from_index, to_index)) => {
                    if let (Some(from), Some(to)) = (
                        app.commit_list.get(from_index),
                        app.commit_list.get(to_index),
                    ) {
                        let job_id = app.next_job();
                        app.worker_req_tx.send(WorkerMsg {
                            job_id,
                            msg: WorkerRequest::CalculateBranchDiff {
                                from_index,
                                to_index,
                                from: from.into(),
                                to: to.into(),
                            },
                        })?;
                        app.current_job = Some(job_id);
                    }
                }
                UiEvent::EnterDiff(usize) => {
                    if let AppScreen::List(ref list_view) = app.screen
                        && let Some(entry) = list_view.get_visible_commits().get(usize)
                    {
                        let job_id = app.next_job();
                        app.worker_req_tx.send(WorkerMsg {
                            job_id,
                            msg: WorkerRequest::RenderInterdiff {
                                from: entry.from.as_ref().map(|e| e.sha.clone()),
                                to: entry.to.as_ref().map(|e| e.sha.clone()),
                                render_width: app.screen_size.0,
                                scroll: 0,
                            },
                        })?;
                        app.current_job = Some(job_id);
                    }
                }
                UiEvent::BackToList => {
                    if let (Some(from), Some(to)) = (
                        app.commit_list.get(app.base_index),
                        app.commit_list.get(app.comparison_index),
                    ) {
                        let job_id = app.next_job();
                        app.worker_req_tx.send(WorkerMsg {
                            job_id,
                            msg: WorkerRequest::CalculateBranchDiff {
                                from_index: app.base_index,
                                to_index: app.comparison_index,
                                from: from.into(),
                                to: to.into(),
                            },
                        })?;
                        app.current_job = Some(job_id);
                    }
                }
                UiEvent::ToggleUnchanged => {
                    if let AppScreen::List(list_view) = &mut app.screen {
                        app.show_unchanged = !app.show_unchanged;
                        list_view.show_unchanged = app.show_unchanged;
                        list_view.list_state.select(Some(0));
                        app.list_state.select(Some(0));
                    }
                }
                UiEvent::CopyToClipboard => {
                    if let (AppScreen::DiffView(diff_view), Ok(mut clipboard)) =
                        (&app.screen, arboard::Clipboard::new())
                    {
                        clipboard.set_text(&diff_view.diff).ok();
                    }
                }
            },
        };

        view_tx.send(app.screen.clone())?;
    }

    // Wait for UI and worker threads to finish
    drop(view_tx);
    ui_handle.join().ok();

    drop(app.worker_req_tx);
    worker_handle.join().ok();

    Ok(())
}
