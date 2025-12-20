use std::sync::mpsc::Sender;

use diffsoup::{
    diff::CommitDiff,
    pr::{PageDirection, Pagination},
};
use jj_lib::ref_name::RefNameBuf;
use ratatui::widgets::ListState;

use crate::tui::{
    JobId,
    worker::{WorkerMsg, WorkerRequest, WorkerResponse},
};

#[derive(Debug)]
pub struct AppState {
    pub screen: AppScreen,
    pub screen_size: (u16, u16),
    pub list_state: ListState,
    pub show_unchanged: bool,
    pub commit_list: Vec<RefNameBuf>,
    pub next_page: Option<Pagination>,
    pub base_index: usize,
    pub comparison_index: usize,
    pub current_job: Option<JobId>,
    pub worker_req_tx: Sender<WorkerMsg<WorkerRequest>>,
}

#[derive(Debug, Clone)]
pub enum AppScreen {
    Loading(Option<String>),
    Exit,
    Error(Option<String>),
    List(ListView),
    DiffView(DiffView),
}

#[derive(Debug, Clone)]
pub struct ListView {
    pub commits: Vec<CommitDiff>,
    pub list_state: ListState,
    pub show_unchanged: bool,
    pub base_name: String,
    pub base_index: usize,
    pub comparison_name: String,
    pub comparison_index: usize,
    pub total_commits: usize,
}

impl ListView {
    pub fn get_visible_commits(&self) -> Vec<&CommitDiff> {
        if self.show_unchanged {
            self.commits.iter().collect()
        } else {
            self.commits.iter().filter(|c| c.has_changes()).collect()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffView {
    pub commit: String,
    pub diff: String,
    pub scroll: u16,
}

#[derive(Debug)]
pub enum UiEvent {
    Exit,
    Scroll(ScrollEvent),
    SizeChange((u16, u16)),
    PatchsetChange((usize, usize)),
    EnterDiff(usize),
    BackToList,
    ToggleUnchanged,
    CopyToClipboard,
}

#[derive(Debug)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug)]
pub enum ScrollEvent {
    Top,
    Single(ScrollDirection),
    Multi((ScrollDirection, u16)),
    HalfPage(ScrollDirection),
    FullPage(ScrollDirection),
    Bottom,
}

impl ScrollEvent {
    pub fn get_new_index(&self, screen_size: (u16, u16), current: usize, length: usize) -> usize {
        let (amount, direction) = match self {
            ScrollEvent::Single(direction) => (1, direction),
            ScrollEvent::Multi((direction, amount)) => (*amount as usize, direction),
            ScrollEvent::HalfPage(direction) => ((screen_size.1 as usize) / 2, direction),
            ScrollEvent::FullPage(direction) => (screen_size.1.into(), direction),
            ScrollEvent::Top => {
                return 0;
            }
            ScrollEvent::Bottom => {
                return length;
            }
        };
        match direction {
            ScrollDirection::Up => current.saturating_sub(amount),
            ScrollDirection::Down => std::cmp::min(length, current.saturating_add(amount)),
        }
    }
}

impl AppState {
    pub fn new(worker_req_tx: Sender<WorkerMsg<WorkerRequest>>) -> Self {
        Self {
            screen: AppScreen::Loading(None),
            screen_size: (0, 0),
            list_state: ListState::default(),
            show_unchanged: false,
            commit_list: Vec::new(),
            next_page: None,
            base_index: 0,
            comparison_index: 0,
            current_job: None,
            worker_req_tx,
        }
    }

    pub fn next_job(&self) -> JobId {
        self.current_job.map(JobId::next).unwrap_or_default()
    }

    pub fn handle_worker(&mut self, response: WorkerResponse) {
        match response {
            WorkerResponse::Error(msg) => self.screen = AppScreen::Error(Some(msg)),
            WorkerResponse::Loading(msg) => self.screen = AppScreen::Loading(Some(msg)),
            WorkerResponse::LoadCommits { page } => {
                let length = page.items.len();
                // insert new at start
                self.commit_list.splice(0..0, page.items);
                match &mut self.screen {
                    AppScreen::Loading(_) => {
                        let job_id = self.next_job();
                        let (from, to) = match page.direction {
                            PageDirection::Backward => {
                                (self.commit_list.len() - 2, self.commit_list.len() - 1)
                            }
                            PageDirection::Forward => (0, 1),
                        };
                        let _ = self.worker_req_tx.send(WorkerMsg {
                            job_id,
                            msg: WorkerRequest::CalculateBranchDiff {
                                from: self.commit_list[from].as_str().to_string(),
                                from_index: from,
                                to: self.commit_list[to].as_str().to_string(),
                                to_index: to,
                            },
                        });
                        self.current_job = Some(job_id);
                    }
                    AppScreen::List(list_view) => {
                        list_view.total_commits = self.commit_list.len();
                        if let Some(pagination) = &self.next_page
                            && matches!(pagination.direction(), PageDirection::Backward)
                        {
                            list_view.base_index += length;
                            list_view.comparison_index += length;
                        }
                    }
                    _ => {}
                }
                self.next_page = page.next;
            }
            WorkerResponse::CalculateBranchDiff { commits, from, to } => {
                self.base_index = from;
                self.comparison_index = to;
                let selected = std::cmp::min(
                    commits.len(),
                    self.list_state.selected().unwrap_or_default(),
                );
                self.screen = AppScreen::List(ListView {
                    list_state: self.list_state.clone().with_selected(Some(selected)),
                    show_unchanged: self.show_unchanged,
                    base_name: self
                        .commit_list
                        .get(from)
                        .map(|c| c.clone().into_string())
                        .unwrap_or_default(),
                    base_index: from,
                    comparison_name: self
                        .commit_list
                        .get(to)
                        .map(|c| c.clone().into_string())
                        .unwrap_or_default(),
                    comparison_index: to,
                    total_commits: self.commit_list.len(),
                    commits,
                });
                let Some(next) = &self.next_page else {
                    return;
                };
                let direction = next.direction();
                let has_next = match direction {
                    PageDirection::Backward => self.base_index == 0,
                    PageDirection::Forward => self.comparison_index >= self.commit_list.len() - 1,
                };
                if has_next {
                    let job_id = self.next_job();
                    let _ = self.worker_req_tx.send(WorkerMsg {
                        job_id,
                        msg: WorkerRequest::LoadCommits {
                            pagination: Some(next.clone()),
                        },
                    });
                    self.current_job = Some(job_id);
                };
            }
            WorkerResponse::RenderInterdiff {
                title,
                diff,
                scroll,
            } => {
                self.screen = AppScreen::DiffView(DiffView {
                    commit: title,
                    diff,
                    scroll,
                });
            }
        }
    }
}
