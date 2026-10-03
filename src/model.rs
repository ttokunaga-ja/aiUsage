use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Chatgpt,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub read: u64,
    pub write: u64,
    pub output: u64,
    pub write_5m: u64,
    pub write_1h: u64,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub provider: String,
    pub tokens: Tokens,
    pub fast: bool,
    pub us_geo: bool,
    /// Full input length of this request, before subtracting cached input.
    pub context_input: u64,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub events: Vec<Event>,
    pub files: usize,
    pub malformed: usize,
    pub excluded: usize,
    pub duplicates: usize,
    pub unstable: usize,
}
