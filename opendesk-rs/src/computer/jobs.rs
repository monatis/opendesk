//! Background commands: start one, poll it for new output and its exit
//! status, or kill its process tree. Jobs outlive the controller's session,
//! which is the point: long batches no longer need a hand-rolled
//! `Start-Process` plus a log file to poll.

use anyhow::{Result, anyhow, bail};
use std::collections::BTreeMap;
use std::time::Duration;

use super::local::{LocalComputer, shell_command};
use super::process::{BackgroundProcess, JobPoll};

/// Running jobs allowed at once per machine.
const MAX_RUNNING: usize = 16;

#[derive(Default)]
pub struct JobTable {
    next_id: u64,
    jobs: BTreeMap<u64, BackgroundProcess>,
}

impl LocalComputer {
    /// Start `command` in the background and return its job id.
    pub fn job_start(
        &self,
        command: &str,
        shell: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<u64> {
        let mut table = self
            .jobs
            .lock()
            .map_err(|_| anyhow!("job table poisoned"))?;
        let mut running = 0;
        for j in table.jobs.values_mut() {
            if j.is_running() {
                running += 1;
            }
        }
        if running >= MAX_RUNNING {
            bail!("{running} background jobs are already running; poll or kill some first");
        }
        let (mut cmd, temp_script) = shell_command(command, shell)?;
        let job = BackgroundProcess::spawn(&mut cmd, command, timeout, temp_script)?;
        table.next_id += 1;
        let id = table.next_id;
        table.jobs.insert(id, job);
        Ok(id)
    }

    /// New output and status. A finished job is forgotten once its final
    /// output has been returned.
    pub fn job_poll(&self, id: u64) -> Result<JobPoll> {
        let mut table = self
            .jobs
            .lock()
            .map_err(|_| anyhow!("job table poisoned"))?;
        let job = table
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("no job {id} (unknown, or already finished and collected)"))?;
        let poll = job.poll(id);
        if !poll.running {
            table.jobs.remove(&id);
        }
        Ok(poll)
    }

    /// Kill a job's process tree and return its final output.
    pub fn job_kill(&self, id: u64) -> Result<JobPoll> {
        {
            let mut table = self
                .jobs
                .lock()
                .map_err(|_| anyhow!("job table poisoned"))?;
            let job = table
                .jobs
                .get_mut(&id)
                .ok_or_else(|| anyhow!("no job {id}"))?;
            job.kill();
        }
        self.job_poll(id)
    }

    /// Every known job's status, without consuming its output.
    pub fn job_list(&self) -> Result<Vec<JobPoll>> {
        let mut table = self
            .jobs
            .lock()
            .map_err(|_| anyhow!("job table poisoned"))?;
        Ok(table
            .jobs
            .iter_mut()
            .map(|(id, j)| JobPoll {
                job_id: *id,
                command: j.command.clone(),
                running: j.is_running(),
                ..Default::default()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_done(c: &LocalComputer, id: u64) -> (String, JobPoll) {
        let mut out = String::new();
        for _ in 0..300 {
            let p = c.job_poll(id).unwrap();
            out.push_str(&p.stdout);
            if !p.running {
                return (out, p);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("job {id} did not finish");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_job_streams_output_and_exit_code() {
        let c = LocalComputer::new();
        let id = c
            .job_start(
                "echo one & ping -n 2 127.0.0.1 >nul & echo two & exit 3",
                Some("cmd"),
                None,
            )
            .unwrap();
        assert!(
            c.job_list()
                .unwrap()
                .iter()
                .any(|j| j.job_id == id && j.running)
        );
        let (out, last) = wait_done(&c, id);
        assert!(out.contains("one") && out.contains("two"), "{out}");
        assert_eq!(last.exit_code, Some(3));
        // Collected after finishing.
        assert!(c.job_poll(id).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn kill_ends_the_whole_tree() {
        let c = LocalComputer::new();
        let id = c
            .job_start(
                "echo started & ping -n 60 127.0.0.1 >nul",
                Some("cmd"),
                None,
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let last = c.job_kill(id).unwrap();
        assert!(last.killed && !last.running, "{last:?}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_job_timeout_kills_it() {
        let c = LocalComputer::new();
        let id = c
            .job_start(
                "ping -n 60 127.0.0.1 >nul",
                Some("cmd"),
                Some(Duration::from_secs(1)),
            )
            .unwrap();
        let (_, last) = wait_done(&c, id);
        assert!(last.timed_out, "{last:?}");
        assert!(last.stderr.contains("timed out"), "{last:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_job_streams_output_and_exit_code() {
        let c = LocalComputer::new();
        let id = c
            .job_start("echo one; sleep 1; echo two; exit 3", Some("sh"), None)
            .unwrap();
        let (out, last) = wait_done(&c, id);
        assert!(out.contains("one") && out.contains("two"), "{out}");
        assert_eq!(last.exit_code, Some(3));
    }
}
