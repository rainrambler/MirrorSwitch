use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Success => "OK",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct AppLog {
    path: PathBuf,
    entries: VecDeque<LogEntry>,
}

impl AppLog {
    pub fn load(path: PathBuf) -> Self {
        let mut entries = VecDeque::new();
        if let Ok(text) = fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            for line in lines.iter().rev().take(250).rev() {
                if let Some(entry) = parse_line(line) {
                    entries.push_back(entry);
                }
            }
        }
        Self { path, entries }
    }

    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        let message = message.into();
        let time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let entry = LogEntry {
            time: time.clone(),
            level,
            message: message.clone(),
        };
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{time} [{}] {message}", level.label());
        }
        self.entries.push_back(entry);
        while self.entries.len() > 300 {
            self.entries.pop_front();
        }
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Info, message);
    }

    pub fn success(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Success, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Error, message);
    }

    #[cfg(test)]
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &LogEntry> {
        self.entries.iter()
    }

    pub fn recent(&self, count: usize) -> Vec<LogEntry> {
        self.entries
            .iter()
            .rev()
            .take(count)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = fs::write(&self.path, "");
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn parse_line(line: &str) -> Option<LogEntry> {
    let (time, rest) = line.split_once(" [")?;
    let (level, message) = rest.split_once("] ")?;
    let level = match level {
        "OK" => LogLevel::Success,
        "WARN" => LogLevel::Warn,
        "ERROR" => LogLevel::Error,
        _ => LogLevel::Info,
    };
    Some(LogEntry {
        time: time.to_owned(),
        level,
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::unique_temp_dir;

    #[test]
    fn writes_and_reloads_entries() {
        let dir = unique_temp_dir("log");
        let path = dir.join("logs").join("app.log");
        let mut log = AppLog::load(path.clone());
        log.success("hello");
        log.error("bad");
        let loaded = AppLog::load(path);
        let entries: Vec<_> = loaded.entries().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].level, LogLevel::Error);
        assert_eq!(entries[1].message, "bad");
        let _ = fs::remove_dir_all(dir);
    }
}
