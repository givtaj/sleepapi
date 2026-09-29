use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::{
    sync::OwnedSemaphorePermit,
    task::{AbortHandle, JoinError},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitMethod {
    #[default]
    TokioSleep,
    TokioSpawn,
    ThreadSleep,
    ThreadPark,
}

impl WaitMethod {
    pub const ALL: [Self; 4] = [
        Self::TokioSleep,
        Self::TokioSpawn,
        Self::ThreadSleep,
        Self::ThreadPark,
    ];

    pub fn description(self) -> &'static str {
        match self {
            Self::TokioSleep => "Await a Tokio timer in the request task; no thread is blocked.",
            Self::TokioSpawn => {
                "Spawn a Tokio task, await its timer, then join it; no thread is blocked."
            }
            Self::ThreadSleep => "Sleep an OS thread in Tokio's blocking pool.",
            Self::ThreadPark => {
                "Park an OS thread in Tokio's blocking pool until the duration elapses."
            }
        }
    }

    pub fn blocks_thread(self) -> bool {
        matches!(self, Self::ThreadSleep | Self::ThreadPark)
    }

    pub(crate) async fn wait(
        self,
        duration: Duration,
        permit: OwnedSemaphorePermit,
    ) -> Result<(), JoinError> {
        match self {
            Self::TokioSleep => {
                let _permit = permit;
                tokio::time::sleep(duration).await;
            }
            Self::TokioSpawn => {
                let task = tokio::spawn(async move {
                    let _permit = permit;
                    tokio::time::sleep(duration).await;
                });
                // Dropping a JoinHandle alone detaches its task. Abort on handler cancellation.
                let _abort = AbortOnDrop(task.abort_handle());
                task.await?;
            }
            Self::ThreadSleep | Self::ThreadPark => {
                wait_in_blocking_pool(
                    move || match self {
                        Self::ThreadSleep => std::thread::sleep(duration),
                        Self::ThreadPark => park_for(duration),
                        _ => unreachable!(),
                    },
                    permit,
                )
                .await?;
            }
        }
        Ok(())
    }
}

async fn wait_in_blocking_pool(
    wait: impl FnOnce() + Send + 'static,
    permit: OwnedSemaphorePermit,
) -> Result<(), JoinError> {
    let task = tokio::task::spawn_blocking(move || {
        // Started blocking tasks cannot be aborted. Keep the capacity slot until done,
        // even if the requesting handler disappears before the work finishes.
        let _permit = permit;
        wait();
    });
    let _abort = AbortOnDrop(task.abort_handle());
    task.await
}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn park_for(duration: Duration) {
    let started = Instant::now();
    // park_timeout may return spuriously or consume a pre-existing unpark token.
    // Recheck elapsed time so either event cannot shorten the requested delay.
    while let Some(remaining) = duration.checked_sub(started.elapsed()) {
        if remaining.is_zero() {
            break;
        }
        std::thread::park_timeout(remaining);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Semaphore;

    use super::*;

    #[test]
    fn parking_rechecks_duration_after_unpark_tokens() {
        let duration = Duration::from_millis(30);
        let parked = std::thread::spawn(move || {
            // A token present before park must not make this return immediately.
            std::thread::current().unpark();
            let started = Instant::now();
            park_for(duration);
            assert!(started.elapsed() >= duration);
        });
        // Also interrupt it repeatedly while it is waiting.
        while !parked.is_finished() {
            parked.thread().unpark();
            std::thread::sleep(Duration::from_millis(1));
        }
        parked.join().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_async_waits_releases_capacity() {
        for method in [WaitMethod::TokioSleep, WaitMethod::TokioSpawn] {
            let slots = Arc::new(Semaphore::new(1));
            let permit = slots.clone().acquire_owned().await.unwrap();
            let request = tokio::spawn(method.wait(Duration::from_secs(60), permit));
            tokio::task::yield_now().await;
            assert_eq!(slots.available_permits(), 0);

            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            // Abort cleanup is scheduled; it need not have run at request.await.
            let recovered =
                tokio::time::timeout(Duration::from_secs(1), slots.clone().acquire_owned())
                    .await
                    .expect("the cancelled async timer must not hold capacity for 60 seconds")
                    .unwrap();
            drop(recovered);
            assert_eq!(slots.available_permits(), 1);
        }
    }

    #[tokio::test]
    async fn cancelling_started_blocking_work_keeps_capacity_until_it_finishes() {
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let request = tokio::spawn(wait_in_blocking_pool(
            move || {
                let _ = started_tx.send(());
                // The test controls completion without guessing how long thread startup takes.
                // Dropping release_tx also releases this thread if an assertion panics.
                let _ = release_rx.blocking_recv();
            },
            permit,
        ));
        started_rx.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert!(slots.clone().try_acquire_owned().is_err());

        release_tx.send(()).unwrap();
        let recovered = tokio::time::timeout(Duration::from_secs(2), slots.acquire_owned())
            .await
            .expect("blocking completion should release capacity")
            .unwrap();
        drop(recovered);
    }
}
