//! A live log stream's history on disk (#914): every line the stream delivers,
//! appended to segment files under a private directory, so the UI keeps only a
//! window in memory and reads older lines back when the reader asks.
//!
//! One JSON record per line, the wire `LogLine` itself; a line's seq is its
//! position, counted from 1 across segments. Each segment keeps the byte offset
//! of every `INDEX_EVERY`th line, so a read seeks close to where it starts and
//! scans at most that many lines. Past `max_bytes` the oldest segment file is
//! deleted and `first` moves on: those lines are gone, and the UI says so.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::logs::LogLine;
use crate::terminal_scope::make_private_dir;

/// A byte offset is kept for every this-many lines of a segment.
const INDEX_EVERY: u64 = 1024;
/// The smallest segment, however small the cap.
const MIN_SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
/// The most lines one read hands back.
pub const MAX_READ_LINES: usize = 5000;
/// The most line text one read hands back. A page past it drops its OLDEST
/// lines, so it still ends where it was asked to and joins what is in memory.
pub const MAX_READ_BYTES: usize = 8 * 1024 * 1024;

/// Lines read back from a spool: `lines[0]` is `seq`, the page ends at the
/// `before` it was asked for, and `first` is the oldest line the spool still has.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpoolPage {
    pub first: u64,
    pub seq: u64,
    pub lines: Vec<LogLine>,
}

struct Segment {
    first: u64,
    path: PathBuf,
    /// Bytes of whole lines written: all a reader may read.
    bytes: u64,
    lines: u64,
    /// `index[k]` is the byte offset of line `first + k * INDEX_EVERY`.
    index: Vec<u64>,
}

/// One stream's history. Not thread-safe on its own: the stream's writer and
/// its readers share it behind a `Mutex`, and a reader holds that lock only
/// for [`LogSpool::snapshot`], never while it reads the files.
pub struct LogSpool {
    dir: PathBuf,
    max_bytes: u64,
    segment_bytes: u64,
    segments: VecDeque<Segment>,
    total: u64,
    next_seq: u64,
    writer: Option<BufWriter<File>>,
    failed: Option<String>,
}

impl LogSpool {
    /// A new spool in `dir`, which must not exist yet: it is created here,
    /// private to this user, and removed when the spool is dropped.
    pub fn create(dir: PathBuf, max_bytes: u64) -> io::Result<Self> {
        Self::create_sized(dir, max_bytes, (max_bytes / 8).max(MIN_SEGMENT_BYTES))
    }

    fn create_sized(dir: PathBuf, max_bytes: u64, segment_bytes: u64) -> io::Result<Self> {
        make_private_dir(&dir)?;
        Ok(Self {
            dir,
            max_bytes,
            segment_bytes,
            segments: VecDeque::new(),
            total: 0,
            next_seq: 1,
            writer: None,
            failed: None,
        })
    }

    /// The oldest seq still on disk: 1 until a segment has been deleted.
    pub fn first(&self) -> u64 {
        self.segments.front().map_or(self.next_seq, |s| s.first)
    }

    /// Why the spool stopped taking lines, once it has.
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Stop taking lines: close the open segment and remember why. Reads say
    /// so from now on; the directory still goes when the spool does.
    pub fn fail(&mut self, reason: String) {
        self.writer = None;
        self.failed.get_or_insert(reason);
    }

    /// Append the next line, returning its seq.
    pub fn append(&mut self, line: &LogLine) -> io::Result<u64> {
        if let Some(reason) = &self.failed {
            return Err(io::Error::other(reason.clone()));
        }
        let mut record = serde_json::to_vec(line).map_err(io::Error::other)?;
        record.push(b'\n');
        if self
            .segments
            .back()
            .is_none_or(|s| s.bytes >= self.segment_bytes)
        {
            self.open_segment()?;
        }
        let segment = self.segments.back_mut().expect("a segment is open");
        let writer = self.writer.as_mut().expect("the newest segment is open");
        writer.write_all(&record)?;
        if segment.lines.is_multiple_of(INDEX_EVERY) {
            segment.index.push(segment.bytes);
        }
        segment.bytes += record.len() as u64;
        segment.lines += 1;
        self.total += record.len() as u64;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.rotate();
        Ok(seq)
    }

    fn open_segment(&mut self) -> io::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        let first = self.next_seq;
        let path = self.dir.join(format!("{first:020}.jsonl"));
        let file = create_private_file(&path)?;
        self.writer = Some(BufWriter::with_capacity(64 * 1024, file));
        self.segments.push_back(Segment {
            first,
            path,
            bytes: 0,
            lines: 0,
            index: Vec::new(),
        });
        Ok(())
    }

    /// Delete the oldest segments while the spool is over its cap: never the
    /// one being written, so a cap smaller than a segment still keeps one.
    fn rotate(&mut self) {
        while self.total > self.max_bytes && self.segments.len() > 1 {
            let old = self.segments.pop_front().expect("more than one segment");
            self.total -= old.bytes;
            let _ = fs::remove_file(&old.path);
        }
    }

    /// What a reader needs, taken under the caller's lock: the open segment
    /// flushed, and each segment's path, length and index as of now. Reading
    /// the files happens after the lock is let go.
    pub fn snapshot(&mut self) -> io::Result<SpoolSnapshot> {
        if let Some(reason) = &self.failed {
            return Err(io::Error::other(reason.clone()));
        }
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        Ok(SpoolSnapshot {
            first: self.first(),
            next: self.next_seq,
            segments: self
                .segments
                .iter()
                .map(|s| SegmentView {
                    first: s.first,
                    path: s.path.clone(),
                    bytes: s.bytes,
                    lines: s.lines,
                    index: s.index.clone(),
                })
                .collect(),
        })
    }
}

impl Drop for LogSpool {
    fn drop(&mut self) {
        // The open segment first: Windows refuses to delete a file still open.
        self.writer = None;
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct SegmentView {
    first: u64,
    path: PathBuf,
    bytes: u64,
    lines: u64,
    index: Vec<u64>,
}

/// A spool as of one moment, readable without its lock. It holds paths, not open
/// files: a segment the writer rotates out after the snapshot is skipped by
/// [`SpoolSnapshot::read`], so a page that would have started in it starts after it.
/// [`SpoolSnapshot::export`] skips such a segment too, but only before it has read
/// any segment: after that, the history would have a hole, so it fails instead.
pub struct SpoolSnapshot {
    pub first: u64,
    next: u64,
    segments: Vec<SegmentView>,
}

impl SpoolSnapshot {
    /// Lines `[max(first, before - limit), before)`: at most
    /// [`MAX_READ_LINES`], and past [`MAX_READ_BYTES`] of text the oldest are
    /// dropped, so the page always ends at `before`. The same goes for lines
    /// whose segment was rotated out since the snapshot: the page starts later.
    /// A segment cut short on disk is an error, not a shorter page.
    pub fn read(&self, before: u64, limit: usize) -> io::Result<SpoolPage> {
        let before = before.min(self.next);
        let start = before
            .saturating_sub(limit.min(MAX_READ_LINES) as u64)
            .max(self.first);
        let mut lines: VecDeque<LogLine> = VecDeque::new();
        let mut bytes = 0usize;
        let mut seq = start;
        for segment in &self.segments {
            if seq >= before {
                break;
            }
            let end = segment.first + segment.lines;
            if end <= seq {
                continue;
            }
            let k = ((seq - segment.first) / INDEX_EVERY) as usize;
            let mut at = segment.first + k as u64 * INDEX_EVERY;
            let mut reader = match open_at(&segment.path, segment.index[k], segment.bytes) {
                Ok(reader) => reader,
                // Rotated out since the snapshot. Segments go oldest first, so
                // whatever came before it is gone too: drop what was gathered, or
                // the page would have a hole, and start the page after it.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    lines.clear();
                    bytes = 0;
                    seq = end;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let mut record = String::new();
            while at < before && at < end {
                read_record(&mut reader, &mut record, &segment.path, at)?;
                if at >= seq {
                    let line: LogLine =
                        serde_json::from_str(record.trim_end()).map_err(io::Error::other)?;
                    bytes += line.line.len();
                    lines.push_back(line);
                    while bytes > MAX_READ_BYTES && lines.len() > 1 {
                        bytes -= lines.pop_front().map_or(0, |l| l.line.len());
                    }
                }
                at += 1;
            }
            seq = at;
        }
        Ok(SpoolPage {
            first: self.first,
            seq: before - lines.len() as u64,
            lines: lines.into(),
        })
    }

    /// Write the history as `<source> | <line>`, the shape the Logs screen's
    /// own export writes, naming each source through `name_of`. With `since`,
    /// only lines stamped at or after it: a line with no stamp of its own (an
    /// `[error: …]` line the host injected) takes the stamp of the line before
    /// it, and a leading run with no stamp at all is kept. Returns how many
    /// lines were written.
    ///
    /// Any line it could not write is an error, never a shorter export: a
    /// segment that cannot be read whole, or one rotated out after the export
    /// has read a segment, fails the call, and the caller must not keep the
    /// file. A segment rotated out before any segment has been read is skipped,
    /// since the export then starts at the oldest segment still on disk.
    pub fn export(
        &self,
        out: &mut dyn Write,
        since: Option<SystemTime>,
        name_of: &dyn Fn(&str) -> String,
    ) -> io::Result<u64> {
        let cutoff = since.map(DateTime::<Utc>::from);
        let mut stamp: Option<DateTime<Utc>> = None;
        let mut written = 0;
        let mut started = false;
        for segment in &self.segments {
            let mut reader = match open_at(&segment.path, 0, segment.bytes) {
                Ok(reader) => reader,
                // Rotated out since the snapshot. Before any segment has been read that
                // is a clean trim of the export's start; after one, the history would
                // have a hole, so it fails.
                Err(e) if e.kind() == io::ErrorKind::NotFound && !started => continue,
                Err(e) => return Err(e),
            };
            started = true;
            let mut record = String::new();
            for seq in segment.first..segment.first + segment.lines {
                read_record(&mut reader, &mut record, &segment.path, seq)?;
                let line: LogLine =
                    serde_json::from_str(record.trim_end()).map_err(io::Error::other)?;
                if let Some(at) = stamp_of(&line.line) {
                    stamp = Some(at);
                }
                let keep = match (cutoff, stamp) {
                    (Some(cut), Some(at)) => at >= cut,
                    _ => true,
                };
                if keep {
                    writeln!(out, "{} | {}", name_of(&line.source), line.line)?;
                    written += 1;
                }
            }
        }
        Ok(written)
    }
}

/// A reader over `path` from `offset` up to `end`: whole lines only, whatever
/// the writer has flushed past them since.
fn open_at(path: &Path, offset: u64, end: u64) -> io::Result<BufReader<io::Take<File>>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    Ok(BufReader::new(file.take(end - offset)))
}

/// The next record of a segment, into `record`. A segment that ends before the
/// line count it was recorded with is an error, not the end of the lines: its
/// index and count promise more than the file holds.
fn read_record(
    reader: &mut impl BufRead,
    record: &mut String,
    path: &Path,
    seq: u64,
) -> io::Result<()> {
    record.clear();
    if reader.read_line(record)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("{} ends before line {seq}", path.display()),
        ));
    }
    Ok(())
}

/// The RFC 3339 stamp the API server puts before a line when timestamps are on.
fn stamp_of(text: &str) -> Option<DateTime<Utc>> {
    let end = text.find(' ').unwrap_or(text.len());
    DateTime::parse_from_rfc3339(&text[..end])
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Created with `O_EXCL` and its final mode, so an existing path is never
/// followed or reused and the file is never briefly readable by others.
fn create_private_file(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// The smallest spool a stream may ask for.
pub const MIN_SPOOL_BYTES: u64 = 16 * 1024 * 1024;
/// The desktop's ceiling: the largest size its setting offers.
pub const DESKTOP_SPOOL_CEILING: u64 = 4 * 1024 * 1024 * 1024;
const ROOT_PREFIX: &str = "srelens-logs-";
const LOCK_FILE: &str = ".lock";

enum Root {
    /// Desktop: a private directory of this process's own under the OS temp dir.
    PerProcess(PathBuf),
    /// Web: a directory the server owns per user and wipes at start.
    Fixed(PathBuf),
}

struct MadeRoot {
    dir: PathBuf,
    /// Held for the process's life, so a later launch's sweep can tell this
    /// root from a dead process's.
    _lock: Option<File>,
}

/// Where a stream manager's spools go and how big one may be. The root is made
/// on first use, so a manager that never spools never touches the disk.
pub struct SpoolDirs {
    root: Root,
    ceiling: u64,
    made: Mutex<Option<MadeRoot>>,
    next: AtomicU64,
}

impl SpoolDirs {
    /// Desktop: one private root per process under `base`, locked while it lives.
    pub fn per_process(base: PathBuf, ceiling: u64) -> Self {
        Self::new(Root::PerProcess(base), ceiling)
    }

    /// Web: spools under `dir`, the user's runtime directory. A `ceiling` of 0
    /// turns spooling off.
    pub fn fixed(dir: PathBuf, ceiling: u64) -> Self {
        Self::new(Root::Fixed(dir), ceiling)
    }

    fn new(root: Root, ceiling: u64) -> Self {
        Self {
            root,
            ceiling,
            made: Mutex::new(None),
            next: AtomicU64::new(0),
        }
    }

    /// The cap a stream gets for asking `requested` bytes, or `None` when
    /// spooling is off.
    pub fn cap(&self, requested: u64) -> Option<u64> {
        (self.ceiling > 0).then(|| requested.clamp(MIN_SPOOL_BYTES.min(self.ceiling), self.ceiling))
    }

    /// A fresh path for one stream's spool, which [`LogSpool::create`] makes.
    /// Named by a counter, never by anything a client sent. The root is made
    /// again if it is gone, whoever deleted it: a sweep that raced this
    /// process's launch, or a temp cleaner.
    pub fn next_stream_dir(&self) -> io::Result<PathBuf> {
        let mut made = self.made.lock().unwrap();
        // ponytail: a sweep can still delete a root between `make_private_dir` and
        // `try_lock`, which costs at most one stream's history, because the next
        // stream finds no directory here and makes the root again.
        if made.as_ref().is_none_or(|m| !m.dir.is_dir()) {
            *made = Some(match &self.root {
                Root::PerProcess(base) => make_process_root(base)?,
                Root::Fixed(dir) => {
                    fs::create_dir_all(dir)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
                    }
                    MadeRoot {
                        dir: dir.clone(),
                        _lock: None,
                    }
                }
            });
        }
        let root = &made.as_ref().expect("made above").dir;
        Ok(root.join(self.next.fetch_add(1, Ordering::Relaxed).to_string()))
    }

    /// Remove the root and everything under it. Best-effort: whatever is left
    /// on the desktop, the next launch's [`sweep_stale`] removes. A later
    /// [`SpoolDirs::next_stream_dir`] makes a fresh root, and the counter keeps
    /// counting, so names never collide.
    pub fn remove_root(&self) {
        // Out of the lock first, so the retries below don't hold it.
        let made = self.made.lock().unwrap().take();
        if let Some(MadeRoot { dir, _lock }) = made {
            drop(_lock);
            // ponytail: a stream's `LogSpool::drop` may be deleting its own directory
            // as this runs, which can fail the sweep of the root half-way. Try again
            // a few times (about 200 ms at the most) while the root is still there,
            // then leave what is left to the next launch's `sweep_stale`.
            for _ in 0..10 {
                if fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn make_process_root(base: &Path) -> io::Result<MadeRoot> {
    for attempt in 0u32..8 {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
            ^ attempt;
        let dir = base.join(format!("{ROOT_PREFIX}{}-{nonce:08x}", std::process::id()));
        match make_private_dir(&dir) {
            Ok(()) => {
                let locked = create_private_file(&dir.join(LOCK_FILE)).and_then(|lock| {
                    lock.try_lock()
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    Ok(lock)
                });
                return match locked {
                    Ok(lock) => Ok(MadeRoot {
                        dir,
                        _lock: Some(lock),
                    }),
                    // The root exists but is not ours to keep: do not leave it behind.
                    Err(e) => {
                        let _ = fs::remove_dir_all(&dir);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(
        "could not make a private directory for log history",
    ))
}

/// Remove the spool roots under `base` whose process is gone: those whose lock
/// this process can take, or that never got one. A running srelens holds its
/// own lock, so its spools stay — dev builds with other identifiers included.
pub fn sweep_stale(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(ROOT_PREFIX) {
            continue;
        }
        let dir = entry.path();
        let stale = match OpenOptions::new().write(true).open(dir.join(LOCK_FILE)) {
            Ok(lock) => lock.try_lock().is_ok(),
            Err(e) => e.kind() == io::ErrorKind::NotFound,
        };
        if stale {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: u64) -> LogLine {
        LogLine {
            source: "web-1".into(),
            line: format!("line {n}"),
            truncated: false,
        }
    }

    fn texts(page: &SpoolPage) -> Vec<String> {
        page.lines.iter().map(|l| l.line.clone()).collect()
    }

    #[test]
    fn numbers_lines_from_one_and_reads_back_the_lines_before_a_seq() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        for n in 1..=10 {
            assert_eq!(spool.append(&line(n)).unwrap(), n);
        }
        let page = spool.snapshot().unwrap().read(8, 3).unwrap();
        assert_eq!((page.first, page.seq), (1, 5));
        assert_eq!(texts(&page), ["line 5", "line 6", "line 7"]);
    }

    #[test]
    fn a_read_stops_at_the_oldest_line_and_at_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        for n in 1..=10 {
            spool.append(&line(n)).unwrap();
        }
        let snapshot = spool.snapshot().unwrap();
        let start = snapshot.read(4, 100).unwrap();
        assert_eq!((start.seq, texts(&start).len()), (1, 3));
        let none = snapshot.read(1, 10).unwrap();
        assert_eq!((none.seq, none.lines.len()), (1, 0));
        let newest = snapshot.read(u64::MAX, 2).unwrap();
        assert_eq!(
            (newest.seq, texts(&newest)),
            (9, vec!["line 9".to_string(), "line 10".to_string()])
        );
    }

    #[test]
    fn a_read_seeks_through_the_index_into_a_long_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        for n in 1..=5000 {
            spool.append(&line(n)).unwrap();
        }
        let page = spool.snapshot().unwrap().read(3001, 10).unwrap();
        assert_eq!(page.seq, 2991);
        assert_eq!(texts(&page).last().unwrap(), "line 3000");
    }

    #[test]
    fn past_its_cap_the_oldest_segment_goes_and_first_moves_on() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        // Tiny segments, so the cap is crossed in a test: ~35 bytes a line.
        let mut spool = LogSpool::create_sized(dir.clone(), 3000, 1000).unwrap();
        for n in 1..=200 {
            spool.append(&line(n)).unwrap();
        }
        let first = spool.first();
        assert!(first > 1, "rotation moved first on, got {first}");
        assert!(
            !dir.join(format!("{:020}.jsonl", 1)).exists(),
            "the oldest file is deleted"
        );
        let snapshot = spool.snapshot().unwrap();
        // Across segment boundaries (rotation leaves `first` near 142), contiguous
        // and ending at `before`.
        let page = snapshot.read(201, 50).unwrap();
        assert_eq!(page.seq, 151);
        assert_eq!(texts(&page).first().unwrap(), "line 151");
        assert_eq!(texts(&page).last().unwrap(), "line 200");
        // Nothing before `first` comes back.
        let gone = snapshot.read(first, 5).unwrap();
        assert!(gone.lines.is_empty());
        assert_eq!(gone.first, first);
    }

    #[test]
    fn a_read_keeps_its_newest_lines_when_it_reaches_the_byte_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        let big = "x".repeat(64 * 1024);
        for n in 1..=130 {
            spool
                .append(&LogLine {
                    source: String::new(),
                    line: format!("{n} {big}"),
                    truncated: false,
                })
                .unwrap();
        }
        let page = spool.snapshot().unwrap().read(131, 5000).unwrap();
        assert!(
            page.lines.len() < 130,
            "the page was cut at {MAX_READ_BYTES} bytes"
        );
        assert_eq!(
            page.seq + page.lines.len() as u64,
            131,
            "and still ends where it was asked to"
        );
        assert!(page.lines.last().unwrap().line.starts_with("130 "));
    }

    #[test]
    fn a_failed_spool_refuses_lines_and_reads_with_its_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        spool.append(&line(1)).unwrap();
        spool.fail("disk full".into());
        assert_eq!(spool.failed(), Some("disk full"));
        assert!(spool.append(&line(2)).is_err());
        assert!(spool
            .snapshot()
            .err()
            .unwrap()
            .to_string()
            .contains("disk full"));
    }

    #[test]
    fn dropping_the_spool_removes_its_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        let mut spool = LogSpool::create(dir.clone(), 64 << 20).unwrap();
        spool.append(&line(1)).unwrap();
        assert!(dir.is_dir());
        drop(spool);
        assert!(!dir.exists());
    }

    #[test]
    fn create_refuses_a_directory_that_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("s")).unwrap();
        assert!(LogSpool::create(tmp.path().join("s"), 64 << 20).is_err());
    }

    /// Several small segments and a roomy cap: the writer has not rotated yet.
    fn spool_of_100_lines_in_small_segments(dir: PathBuf) -> LogSpool {
        let mut spool = LogSpool::create_sized(dir, 64 << 20, 1000).unwrap();
        for n in 1..=100 {
            spool.append(&line(n)).unwrap();
        }
        spool
    }

    #[test]
    fn a_read_skips_a_segment_rotated_out_after_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        assert!(
            snapshot.segments.len() > 2,
            "the test needs several segments"
        );
        // The writer rotates the oldest segment out after the reader's snapshot.
        let second = snapshot.segments[1].first;
        fs::remove_file(&snapshot.segments[0].path).unwrap();
        let page = snapshot.read(101, 100).unwrap();
        assert_eq!(
            page.seq, second,
            "the page starts where the rotated-out segment ended"
        );
        assert_eq!(
            page.seq + page.lines.len() as u64,
            101,
            "and still ends where it was asked to"
        );
        assert_eq!(texts(&page).first().unwrap(), &format!("line {second}"));
        assert_eq!(texts(&page).last().unwrap(), "line 100");
    }

    #[test]
    fn a_read_drops_what_it_gathered_when_a_newer_segment_goes_under_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        // Rotation evicts oldest-first, so a reader that has already read segment 0
        // can find segment 1 gone only because segment 0 went too: what it read
        // must not stay, or the page would have a hole before segment 2.
        let third = snapshot.segments[2].first;
        fs::remove_file(&snapshot.segments[1].path).unwrap();
        let page = snapshot.read(101, 100).unwrap();
        assert_eq!(page.seq, third, "the page starts after the missing segment");
        assert_eq!(page.seq + page.lines.len() as u64, 101);
        assert_eq!(texts(&page).first().unwrap(), &format!("line {third}"));
    }

    #[test]
    fn a_read_of_a_segment_cut_short_is_an_error_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        // Something outside the spool (a temp cleaner) cuts the oldest segment
        // after its first record: the index and the line count now promise more.
        let path = &snapshot.segments[0].path;
        let first_record = fs::read(path)
            .unwrap()
            .iter()
            .position(|b| *b == b'\n')
            .unwrap() as u64
            + 1;
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(first_record)
            .unwrap();
        let err = snapshot
            .read(101, 100)
            .expect_err("a short segment is an error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_and_its_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("s");
        let mut spool = LogSpool::create(dir.clone(), 64 << 20).unwrap();
        spool.append(&line(1)).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(format!("{:020}.jsonl", 1))), 0o600);
    }

    fn lines_of(spool: &mut LogSpool, lines: &[(&str, &str)]) {
        for (source, text) in lines {
            spool
                .append(&LogLine {
                    source: (*source).into(),
                    line: (*text).into(),
                    truncated: false,
                })
                .unwrap();
        }
    }

    fn named(source: &str) -> String {
        if source.is_empty() {
            "web-1".into()
        } else {
            source.into()
        }
    }

    #[test]
    fn an_export_writes_source_and_line_naming_an_untagged_source() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        lines_of(
            &mut spool,
            &[
                ("", "2026-10-13T10:00:00Z boot"),
                ("web-2/app", "2026-10-13T10:05:00.5Z ready"),
            ],
        );
        let mut out = Vec::new();
        let n = spool
            .snapshot()
            .unwrap()
            .export(&mut out, None, &named)
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "web-1 | 2026-10-13T10:00:00Z boot\nweb-2/app | 2026-10-13T10:05:00.5Z ready\n"
        );
    }

    #[test]
    fn an_export_since_a_time_keeps_lines_stamped_at_or_after_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create(tmp.path().join("s"), 64 << 20).unwrap();
        lines_of(
            &mut spool,
            &[
                ("", "[error: before any stamp]"),     // a leading run: kept
                ("", "2026-10-13T10:00:00Z old"),      // before the cutoff
                ("", "[error: after old]"),            // inherits 10:00: dropped
                ("", "2026-10-13T12:10:00+02:00 new"), // 10:10 UTC: kept
                ("", "[error: after new]"),            // inherits 10:10: kept
            ],
        );
        let since: std::time::SystemTime =
            chrono::DateTime::parse_from_rfc3339("2026-10-13T10:05:00Z")
                .unwrap()
                .into();
        let mut out = Vec::new();
        spool
            .snapshot()
            .unwrap()
            .export(&mut out, Some(since), &named)
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "web-1 | [error: before any stamp]\nweb-1 | 2026-10-13T12:10:00+02:00 new\nweb-1 | [error: after new]\n"
        );
    }

    #[test]
    fn an_export_of_a_segment_cut_short_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        // The same cut as the read test: the oldest segment ends after its first record.
        let path = &snapshot.segments[0].path;
        let first_record = fs::read(path)
            .unwrap()
            .iter()
            .position(|b| *b == b'\n')
            .unwrap() as u64
            + 1;
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(first_record)
            .unwrap();
        let mut out = Vec::new();
        let err = snapshot
            .export(&mut out, None, &named)
            .expect_err("a short segment is an error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn an_export_skips_a_leading_segment_rotated_out_before_any_segment_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        // The writer rotates the oldest segment out after the snapshot. No segment
        // has been read yet, so the export starts at the oldest segment still on disk.
        let second = snapshot.segments[1].first;
        fs::remove_file(&snapshot.segments[0].path).unwrap();
        let mut out = Vec::new();
        let n = snapshot.export(&mut out, None, &named).unwrap();
        assert_eq!(n, 101 - second);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(&format!("web-1 | line {second}\n")));
        assert!(text.ends_with("web-1 | line 100\n"));
    }

    #[test]
    fn an_export_fails_when_a_segment_goes_after_a_segment_was_read() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = spool_of_100_lines_in_small_segments(tmp.path().join("s"));
        let snapshot = spool.snapshot().unwrap();
        // Rotation evicts oldest-first, so segment 1 can only go once segment 0 is
        // read: the export would then have a hole before segment 2.
        fs::remove_file(&snapshot.segments[1].path).unwrap();
        let mut out = Vec::new();
        let err = snapshot
            .export(&mut out, None, &named)
            .expect_err("a hole is an error");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn an_export_fails_when_a_segment_goes_after_a_read_that_wrote_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut spool = LogSpool::create_sized(tmp.path().join("s"), 64 << 20, 1000).unwrap();
        for n in 1..=40 {
            spool
                .append(&LogLine {
                    source: "web-1".into(),
                    line: format!("2026-10-13T10:00:{n:02}Z line {n}"),
                    truncated: false,
                })
                .unwrap();
        }
        let snapshot = spool.snapshot().unwrap();
        // Line n is stamped n seconds past 10:00. The cutoff falls inside segment 1,
        // so segment 0 is read in full and every line of it is dropped: nothing is
        // written before segment 1 goes.
        let cut = snapshot.segments[1].first + 2;
        let since: std::time::SystemTime =
            chrono::DateTime::parse_from_rfc3339(&format!("2026-10-13T10:00:{cut:02}Z"))
                .unwrap()
                .into();
        fs::remove_file(&snapshot.segments[1].path).unwrap();
        let mut out = Vec::new();
        let err = snapshot
            .export(&mut out, Some(since), &named)
            .expect_err("a segment read before the vanish leaves a hole");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    const MIB: u64 = 1024 * 1024;

    fn only_root(base: &Path) -> PathBuf {
        let mut roots: Vec<_> = fs::read_dir(base)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(roots.len(), 1, "{roots:?}");
        roots.pop().unwrap()
    }

    #[test]
    fn a_cap_is_clamped_to_the_ceiling_and_zero_turns_spooling_off() {
        let desktop = SpoolDirs::per_process(PathBuf::from("unused"), DESKTOP_SPOOL_CEILING);
        assert_eq!(desktop.cap(1), Some(MIN_SPOOL_BYTES));
        assert_eq!(desktop.cap(256 * MIB), Some(256 * MIB));
        assert_eq!(desktop.cap(10 * 1024 * MIB), Some(DESKTOP_SPOOL_CEILING));
        assert_eq!(
            SpoolDirs::fixed(PathBuf::from("unused"), 0).cap(256 * MIB),
            None
        );
        let small = SpoolDirs::fixed(PathBuf::from("unused"), 8 * MIB);
        assert_eq!(small.cap(256 * MIB), Some(8 * MIB));
        assert_eq!(small.cap(1), Some(8 * MIB));
    }

    #[test]
    fn a_process_gets_one_private_root_and_numbered_stream_dirs_in_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING);
        let a = dirs.next_stream_dir().unwrap();
        let b = dirs.next_stream_dir().unwrap();
        let root = only_root(tmp.path());
        let name = root.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(&format!("srelens-logs-{}-", std::process::id())),
            "{name}"
        );
        assert_eq!((a, b), (root.join("0"), root.join("1")));
        assert!(
            !root.join("0").exists(),
            "the spool makes its own directory"
        );
        dirs.remove_root();
        assert!(!root.exists());
    }

    #[test]
    fn a_fixed_root_is_made_on_first_use() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = SpoolDirs::fixed(tmp.path().join("logs"), 256 * MIB);
        assert!(!tmp.path().join("logs").exists());
        assert_eq!(
            dirs.next_stream_dir().unwrap(),
            tmp.path().join("logs").join("0")
        );
        assert!(tmp.path().join("logs").is_dir());
    }

    #[test]
    fn a_root_deleted_under_a_running_process_is_remade_for_the_next_stream() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING);
        let first = dirs.next_stream_dir().unwrap();
        let gone = first.parent().unwrap().to_path_buf();
        // A sweep (or a temp cleaner) deletes the root while this process runs.
        fs::remove_dir_all(&gone).unwrap();

        let next = dirs.next_stream_dir().unwrap();

        let root = next.parent().unwrap();
        assert!(root.is_dir(), "the next stream gets a root that exists");
        assert_ne!(root, gone, "a fresh root, not the deleted one");
        // And a spool can be made in it: the failure this guards against.
        LogSpool::create(next, 64 << 20).unwrap();
    }

    #[test]
    fn the_sweep_removes_dead_roots_and_keeps_a_live_one() {
        let tmp = tempfile::tempdir().unwrap();
        let live = SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING);
        live.next_stream_dir().unwrap();
        let live_root = only_root(tmp.path());
        // A dead process's root: its lock file is there, and nobody holds it.
        let dead = tmp.path().join("srelens-logs-1-deadbeef");
        fs::create_dir(&dead).unwrap();
        fs::write(dead.join(".lock"), b"").unwrap();
        // Killed between making the root and its lock.
        let bare = tmp.path().join("srelens-logs-2-cafef00d");
        fs::create_dir(&bare).unwrap();
        let other = tmp.path().join("something-else");
        fs::create_dir(&other).unwrap();

        sweep_stale(tmp.path());

        assert!(live_root.is_dir(), "a running process keeps its spools");
        assert!(!dead.exists());
        assert!(!bare.exists());
        assert!(other.is_dir(), "only srelens-logs-* is ours to sweep");
    }

    #[cfg(unix)]
    #[test]
    fn the_process_root_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dirs = SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING);
        dirs.next_stream_dir().unwrap();
        let mode = fs::metadata(only_root(tmp.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}
