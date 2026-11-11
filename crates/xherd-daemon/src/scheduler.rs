//! One task owns every periodic job (design §3.5). Jobs send messages to the
//! supervisor and actors; they never change agents directly.

use std::time::Duration;

use tokio::time::{MissedTickBehavior, interval};

use crate::Daemon;
use crate::actor::TickKind;

#[derive(Debug, Clone)]
pub struct Intervals {
    pub screen: Duration,
    pub poll: Duration,
    pub minute: Duration,
    pub daily: Duration,
}

impl Default for Intervals {
    fn default() -> Intervals {
        Intervals {
            screen: Duration::from_secs(3),
            poll: Duration::from_secs(30),
            minute: Duration::from_secs(60),
            daily: Duration::from_secs(24 * 3600),
        }
    }
}

pub fn spawn(d: Daemon, iv: Intervals) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mk = |p: Duration| {
            let mut i = interval(p);
            i.set_missed_tick_behavior(MissedTickBehavior::Delay);
            i
        };
        let (mut screen, mut poll, mut minute, mut daily) =
            (mk(iv.screen), mk(iv.poll), mk(iv.minute), mk(iv.daily));
        loop {
            tokio::select! {
                _ = screen.tick() => d.sup.tick(TickKind::ScreenScan),
                _ = poll.tick() => {
                    d.sup.tick(TickKind::TodoTick);
                    d.todo_tick().await;
                    d.sup.tick(TickKind::PrPoll);
                }
                _ = minute.tick() => {
                    d.reconcile().await;
                    d.sup.tick(TickKind::IdleCheck);
                    d.sup.tick(TickKind::ParkCheck);
                    d.sup.tick(TickKind::SessionLimit);
                    // Reports queued while the daemon was unreachable but running.
                    if let Err(e) = d.replay_queue().await {
                        tracing::warn!(error = %e, "hook queue replay failed");
                    }
                    d.flush_worklog().await;
                }
                _ = daily.tick() => {
                    d.prune_events().await;
                    // Scheduled backups (M4) and the update check (R-1303) hook in here.
                }
            }
        }
    })
}
