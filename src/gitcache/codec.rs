use super::{Cache, DayEntry, TagEntry};
use anyhow::{anyhow, bail, Result};
use time::{macros::date, Date, Duration};

const VERSION: u8 = 1;
const EPOCH: Date = date!(1980 - 01 - 01);

fn days_from_epoch(d: Date) -> u16 {
    let days = (d - EPOCH).whole_days();
    if days < 0 {
        0
    } else if days > u16::MAX as i64 {
        u16::MAX
    } else {
        days as u16
    }
}

fn days_to_date(days: u16) -> Date {
    EPOCH + Duration::days(days as i64)
}

pub fn encode(cache: &Cache) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.push(VERSION);
    buf.extend_from_slice(&days_from_epoch(cache.last_updated).to_le_bytes());
    buf.extend_from_slice(&days_from_epoch(cache.oldest_commit).to_le_bytes());
    buf.extend_from_slice(&days_from_epoch(cache.first_entry).to_le_bytes());

    let n_authors: u16 = cache
        .authors
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many authors"))?;
    buf.extend_from_slice(&n_authors.to_le_bytes());
    for a in &cache.authors {
        buf.extend_from_slice(a.as_bytes());
        buf.push(0);
    }

    let n_tags: u16 = cache
        .tags
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many tags"))?;
    buf.extend_from_slice(&n_tags.to_le_bytes());
    for t in &cache.tags {
        buf.extend_from_slice(&t.delta.to_le_bytes());
        buf.extend_from_slice(&t.count.to_le_bytes());
    }

    let n_entries: u16 = cache
        .entries
        .len()
        .try_into()
        .map_err(|_| anyhow!("too many entries"))?;
    buf.extend_from_slice(&n_entries.to_le_bytes());
    for e in &cache.entries {
        buf.extend_from_slice(&e.delta.to_le_bytes());
        buf.extend_from_slice(&e.commits.to_le_bytes());
        buf.extend_from_slice(&e.merges.to_le_bytes());
        let n: u16 = e
            .authors
            .len()
            .try_into()
            .map_err(|_| anyhow!("too many entry authors"))?;
        buf.extend_from_slice(&n.to_le_bytes());
        for id in &e.authors {
            buf.extend_from_slice(&id.to_le_bytes());
        }
    }
    Ok(buf)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).ok_or_else(|| anyhow!("unexpected end"))?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16> {
        let end = self.pos + 2;
        let s = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("unexpected end"))?;
        self.pos = end;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    fn cstr(&mut self) -> Result<String> {
        let start = self.pos;
        while *self
            .data
            .get(self.pos)
            .ok_or_else(|| anyhow!("unterminated string"))?
            != 0
        {
            self.pos += 1;
        }
        let s = String::from_utf8(self.data[start..self.pos].to_vec())?;
        self.pos += 1;
        Ok(s)
    }
}

pub fn decode(data: &[u8]) -> Result<Cache> {
    let mut r = Reader { data, pos: 0 };
    let ver = r.u8()?;
    if ver != VERSION {
        bail!("unsupported cache version {ver}");
    }

    let last_updated = days_to_date(r.u16()?);
    let oldest_commit = days_to_date(r.u16()?);
    let first_entry = days_to_date(r.u16()?);

    let n_authors = r.u16()?;
    let mut authors = Vec::with_capacity(n_authors as usize);
    for _ in 0..n_authors {
        authors.push(r.cstr()?);
    }

    let n_tags = r.u16()?;
    let mut tags = Vec::with_capacity(n_tags as usize);
    for _ in 0..n_tags {
        tags.push(TagEntry {
            delta: r.u16()?,
            count: r.u16()?,
        });
    }

    let n_entries = r.u16()?;
    let mut entries = Vec::with_capacity(n_entries as usize);
    for _ in 0..n_entries {
        let delta = r.u16()?;
        let commits = r.u16()?;
        let merges = r.u16()?;
        let n = r.u16()?;
        let mut a = Vec::with_capacity(n as usize);
        for _ in 0..n {
            a.push(r.u16()?);
        }
        entries.push(DayEntry {
            delta,
            commits,
            merges,
            authors: a,
        });
    }

    Ok(Cache {
        last_updated,
        oldest_commit,
        first_entry,
        authors,
        entries,
        tags,
    })
}
