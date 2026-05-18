use time::Date;

pub mod build;
pub mod codec;
pub mod derive;
pub mod paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    pub last_updated: Date,
    pub oldest_commit: Date,
    pub first_entry: Date,
    pub authors: Vec<String>,
    pub entries: Vec<DayEntry>,
    pub tags: Vec<TagEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayEntry {
    pub delta: u16,
    pub commits: u16,
    pub merges: u16,
    pub authors: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    pub delta: u16,
    pub count: u16,
}
