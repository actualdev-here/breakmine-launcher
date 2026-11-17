//! Track launcher-owned children and recover matching orphaned Linux processes.
use std::{path::{Path, PathBuf}, process::{Child, Command}, time::{Duration, Instant}};

pub enum ManagedProcess {
    Child(Child),
    #[cfg(target_os = "linux")]
    Recovered { pid: u32, started: u64, args: Vec<String>, cwd: Option<PathBuf> },
}

#[cfg(target_os = "linux")]
fn details(pid: u32) -> Option<(u32, u64, Vec<String>, PathBuf)> {
    let base = PathBuf::from(format!("/proc/{pid}"));
    let stat = std::fs::read_to_string(base.join("stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if fields.first()? == &"Z" { return None; }
    let parent = fields.get(1)?.parse().ok()?;
    let started = fields.get(19)?.parse().ok()?;
    let args = std::fs::read(base.join("cmdline")).ok()?.split(|b| *b == 0)
        .filter(|b| !b.is_empty()).map(|b| String::from_utf8_lossy(b).into_owned()).collect();
    let cwd = std::fs::read_link(base.join("cwd")).ok()?;
    Some((parent, started, args, cwd))
}

impl ManagedProcess {
    pub fn recover(args: &[String], cwd: Option<&Path>) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let matches: Vec<_> = std::fs::read_dir("/proc").ok()?.flatten().filter_map(|entry| {
                let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
                let (parent, started, actual, actual_cwd) = details(pid)?;
                if parent != 1 || actual != args || cwd.is_some_and(|p| p != actual_cwd) { return None; }
                Some(Self::Recovered { pid, started, args: args.to_vec(), cwd: cwd.map(Path::to_path_buf) })
            }).collect();
            if matches.len() == 1 { return matches.into_iter().next(); }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (args, cwd);
        None
    }
    pub fn alive(&mut self) -> bool {
        match self {
            Self::Child(child) => child.try_wait().map(|s| s.is_none()).unwrap_or(true),
            #[cfg(target_os = "linux")]
            Self::Recovered { pid, started, args, cwd } => details(*pid).is_some_and(|(_, time, actual, actual_cwd)| time == *started && actual == *args && cwd.as_ref().is_none_or(|p| *p == actual_cwd)),
        }
    }
    pub fn stop(&mut self, timeout: Duration) -> Result<(), String> {
        if !self.alive() { return Ok(()); }
        match self {
            Self::Child(child) => {
                #[cfg(unix)]
                { let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status(); }
                #[cfg(not(unix))]
                { let _ = child.kill(); }
            }
            #[cfg(target_os = "linux")]
            Self::Recovered { pid, .. } => {
                let result = Command::new("kill").args(["-TERM", &pid.to_string()]).status().map_err(|e| e.to_string())?;
                if !result.success() { return Err("Couldn't stop the recovered process.".into()); }
            }
        }
        let start = Instant::now();
        while self.alive() {
            if start.elapsed() > timeout {
                return Err("Process did not stop in time. Check it before restarting.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
