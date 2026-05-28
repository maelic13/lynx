use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    mpsc::{self, Sender},
};
use std::thread::{self, JoinHandle};

use crate::board::{Board, Move};
use crate::search::{SearchEvent, SearchResult, Searcher};
use crate::search_options::{EngineOptions, SearchLimits};
use crate::tt::TranspositionTable;

pub(crate) const STOP_NONE: u8 = 0;
pub(crate) const STOP_SEARCH: u8 = 1;
pub(crate) const STOP_QUIT: u8 = 2;

pub(crate) struct SharedSearchState {
    pub stop_state: AtomicU8,
    pub ponderhit: AtomicBool,
    pub nodes: AtomicU64,
    pub tb_hits: AtomicU64,
}

impl SharedSearchState {
    pub(crate) fn new(initial_tb_hits: u64) -> Self {
        Self {
            stop_state: AtomicU8::new(STOP_NONE),
            ponderhit: AtomicBool::new(false),
            nodes: AtomicU64::new(0),
            tb_hits: AtomicU64::new(initial_tb_hits),
        }
    }

    pub(crate) fn request_stop(&self) {
        let _ = self
            .stop_state
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |state| {
                (state != STOP_QUIT).then_some(STOP_SEARCH)
            });
    }

    pub(crate) fn request_quit(&self) {
        self.stop_state.store(STOP_QUIT, Ordering::Relaxed);
    }
}

pub(crate) struct WorkerJob {
    pub root: Board,
    pub root_moves: Arc<[Move]>,
    pub limits: SearchLimits,
    pub engine_options: EngineOptions,
    pub tt: TranspositionTable,
    pub hash_mb: usize,
    pub root_move_offset: usize,
    pub shared_state: Arc<SharedSearchState>,
    pub result_tx: Sender<SearchResult>,
}

enum WorkerMessage {
    Search(WorkerJob),
    NewGame,
    Shutdown,
}

struct SearchWorkerHandle {
    sender: Sender<WorkerMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(crate) struct WorkerPool {
    workers: Vec<SearchWorkerHandle>,
}

impl WorkerPool {
    pub(crate) fn set_helper_count(&mut self, helper_count: usize) {
        while self.workers.len() > helper_count {
            if let Some(mut worker) = self.workers.pop() {
                let _ = worker.sender.send(WorkerMessage::Shutdown);
                if let Some(handle) = worker.handle.take() {
                    let _ = handle.join();
                }
            }
        }
        while self.workers.len() < helper_count {
            if let Some(worker) = spawn_search_worker(self.workers.len()) {
                self.workers.push(worker);
            } else {
                println!(
                    "info string Unable to create helper search thread {}; using {} search threads.",
                    self.workers.len() + 1,
                    self.workers.len() + 1
                );
                break;
            }
        }
    }

    pub(crate) fn new_game(&self) {
        for worker in &self.workers {
            let _ = worker.sender.send(WorkerMessage::NewGame);
        }
    }

    pub(crate) fn send_search(&self, index: usize, job: WorkerJob) -> bool {
        self.workers
            .get(index)
            .is_some_and(|worker| worker.sender.send(WorkerMessage::Search(job)).is_ok())
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.set_helper_count(0);
    }
}

fn spawn_search_worker(index: usize) -> Option<SearchWorkerHandle> {
    let (sender, receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("lynx-search-{index}"))
        .spawn(move || {
            let mut worker = Searcher::worker_default();
            while let Ok(message) = receiver.recv() {
                match message {
                    WorkerMessage::Search(job) => {
                        let result_tx = job.result_tx.clone();
                        let shared_state = Arc::clone(&job.shared_state);
                        let mut helper_poll =
                            || match shared_state.stop_state.load(Ordering::Relaxed) {
                                STOP_QUIT => SearchEvent::Quit,
                                STOP_SEARCH => SearchEvent::Stop,
                                _ if shared_state.ponderhit.load(Ordering::Relaxed) => {
                                    SearchEvent::PonderHit
                                }
                                _ => SearchEvent::None,
                            };
                        let result = worker.run_worker_job(job, &mut helper_poll);
                        let _ = result_tx.send(result);
                    }
                    WorkerMessage::NewGame => worker.reset_worker_state_for_new_game(),
                    WorkerMessage::Shutdown => break,
                }
            }
        })
        .ok()?;
    Some(SearchWorkerHandle {
        sender,
        handle: Some(handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_stop_request_sets_search_stop() {
        let state = SharedSearchState::new(0);

        state.request_stop();

        assert_eq!(state.stop_state.load(Ordering::Relaxed), STOP_SEARCH);
    }

    #[test]
    fn shared_stop_request_does_not_overwrite_quit() {
        let state = SharedSearchState::new(0);

        state.request_quit();
        state.request_stop();

        assert_eq!(state.stop_state.load(Ordering::Relaxed), STOP_QUIT);
    }
}
