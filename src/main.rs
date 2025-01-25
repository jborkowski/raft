use std::{fmt::Display, time::Duration};

use tokio::{
    sync::{watch, Mutex},
    time::Instant,
};

fn main() {
    println!("Hello, world!");
}

#[allow(dead_code)]
enum CMState {
    Follower,
    Candidate,
    Leader,
    Dead,
}

impl Display for CMState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CMState::Follower => write!(f, "Follower"),
            CMState::Candidate => write!(f, "Candidate"),
            CMState::Leader => write!(f, "Leader"),
            CMState::Dead => write!(f, "Dead"),
        }
    }
}

#[allow(dead_code)]
type Command = ();

struct LogEntry {
    command: Command,
    term: usize,
}

struct ConsensusModule {
    // Persistent state on all servers
    current_term: Mutex<usize>,
    voted_for: Option<usize>,
    log: Vec<LogEntry>,

    // Volatile state on all servers
    commit_index: usize,
    last_applied: usize,

    // Volatile state on leaders (reinitialized after election)
    // for each server, index of next log entry to send
    next_index: Vec<usize>,
    // for each server, index of highest log entry known to be replicated
    match_index: Vec<usize>,

    // Additional state
    state: Mutex<CMState>,
    id: usize,
    peers: Vec<usize>,
    election_timeout: std::time::Duration,
    last_heartbeat: Mutex<Instant>,
}

impl ConsensusModule {
    pub fn new(id: usize, peers: Vec<usize>) -> Self {
        ConsensusModule {
            current_term: Mutex::new(0),
            voted_for: None,
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
            next_index: vec![0; peers.len()],
            match_index: vec![0; peers.len()],
            state: Mutex::new(CMState::Follower),
            id,
            peers,
            // random timeout between 150-300ms
            election_timeout: std::time::Duration::from_millis(150 + rand::random::<u64>() % 150),
            last_heartbeat: Mutex::new(Instant::now()),
        }
    }

    pub async fn is_leader(&self) -> bool {
        let state = self.state.lock().await;
        matches!(*state, CMState::Leader)
    }

    pub async fn update_heartbeat(&self) {
        let mut heartbeat = self.last_heartbeat.lock().await;
        *heartbeat = Instant::now()
    }

    pub async fn inc_term(&self) {
        let mut term = self.current_term.lock().await;
        *term += 1
    }

    pub async fn update_state(&self, new_state: CMState) {
        let mut state = self.state.lock().await;
        *state = new_state
    }

    pub async fn run_election_timer(&mut self, mut shutdown_rx: watch::Receiver<bool>) {
        loop {
            self.election_timeout = Duration::from_millis(150 + (rand::random::<u64>() % 150));

            self.update_heartbeat().await;

            let mut timeout = Box::pin(tokio::time::sleep(self.election_timeout));

            tokio::select! {
                _ = &mut timeout => {
                    // Election timeout expired
                    let heartbeat = self.last_heartbeat.lock().await;
                    if heartbeat.elapsed() >= self.election_timeout {
                        // Transition to candidate
                        self.update_state(CMState::Candidate).await;
                        self.inc_term().await;
                        self.voted_for = Some(self.id);

                        // TODO: Trigger new election process
                        break;
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                      println!("Node {} shuting down election timer.", self.id)
                    }
                }
            }
        }
    }

    pub async fn receive_heartbeat(&mut self) {
        self.update_heartbeat().await;
        let state = self.state.lock().await;
        if matches!(*state, CMState::Candidate) {
            self.update_state(CMState::Follower).await;
            println!(
                "Node {} transitioning back to Follower due to heartbeat.",
                self.id
            );
        }
    }
}

struct Server {}

impl Server {
    pub fn new() {}
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::watch;

    use crate::*;

    #[tokio::test]
    async fn test_election_timer_shutdown() {
        let mut cm = ConsensusModule::new(1, vec![2, 3]);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let timer_handle = tokio::spawn(async move {
            cm.run_election_timer(shutdown_rx).await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        shutdown_tx
            .send(true)
            .expect("Failed to send shutdown signal");

        let _ = tokio::time::timeout(Duration::from_millis(100), timer_handle).await;
    }

    #[tokio::test]
    async fn test_trigger_new_election() {
        let mut cm = ConsensusModule::new(1, vec![2, 3]);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        assert!(matches!(*cm.state.lock().await, CMState::Follower));
        assert_eq!(*cm.current_term.lock().await, 0);
        assert_eq!(cm.voted_for, None);

        let timer_handle = tokio::spawn(async move {
            cm.run_election_timer(shutdown_rx).await;
            cm
        });

        // Wait for election timeout and state transition
        tokio::time::sleep(Duration::from_millis(400)).await;

        // try to send shutdown signal but do not panic if timer has already completed
        let _ = shutdown_tx.send(true);

        let cm = timer_handle.await.expect("Timer task failed");

        assert!(matches!(*cm.state.lock().await, CMState::Candidate));
        assert_eq!(cm.voted_for, Some(1));

        assert_eq!(*cm.current_term.lock().await, 1);
    }
}
