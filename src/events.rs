use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSource {
    Jira,
    GitHub,
}

impl fmt::Display for EventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventSource::Jira => write!(f, "Jira"),
            EventSource::GitHub => write!(f, "GitHub"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventLevel {
    Neutral,
    Info,
    Success,
    Warning,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Event {
    /// ISO 8601 timestamp string for sorting
    pub at: String,
    pub source: EventSource,
    pub level: EventLevel,
    pub title: String,
    pub detail: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum EventLoadState {
    NotLoaded,
    Loading,
    Loaded(Vec<Event>),
    Error(String),
}
