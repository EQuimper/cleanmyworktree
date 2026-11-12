//! Repo-wide advisory lock so parallel agents don't race (e.g. `gc` removing a worktree while
//! `new` creates one). Lives at `<git-common-dir>/cmw.lock`; released when the guard drops.

use crate::output::{CmwError, Result};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct RepoLock {
    file: File,
}

impl Drop for RepoLock {
    fn drop(&mut self) {
        // SAFETY: valid fd owned by self.file.
        #[cfg(unix)]
        unsafe {
            use std::os::unix::io::AsRawFd;
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// Acquire the lock, waiting up to `timeout`. Errors with a hint if another cmw holds it.
pub fn acquire(common_dir: &Path, timeout: Duration) -> Result<RepoLock> {
    let path = common_dir.join("cmw.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    let start = Instant::now();
    loop {
        if try_lock(&file) {
            return Ok(RepoLock { file });
        }
        if start.elapsed() >= timeout {
            return Err(CmwError::error(format!(
                "another cmw process holds {} (waited {}s)",
                path.display(),
                timeout.as_secs()
            ))
            .with_hint("retry in a few seconds; if no cmw is running, the lock is stale and the retry will succeed"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    // SAFETY: valid fd. flock locks are released by the kernel if the process dies, so they never go stale.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

#[cfg(not(unix))]
fn try_lock(_file: &File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_times_out_then_succeeds_after_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let g = acquire(tmp.path(), Duration::from_millis(50)).unwrap();
        assert!(acquire(tmp.path(), Duration::from_millis(150)).is_err());
        drop(g);
        assert!(acquire(tmp.path(), Duration::from_millis(50)).is_ok());
    }
}
