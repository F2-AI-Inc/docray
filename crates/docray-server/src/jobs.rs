use docray_core::PageSelection;
use docray_model::{Granularity, OutputFormat};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

pub struct JobStore {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct JobRow {
    pub id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub result_path: Option<String>,
    pub format: String,
}

pub struct ClaimedJob {
    pub id: String,
    pub input_path: String,
    pub granularity: Option<Granularity>,
    pub format: OutputFormat,
    pub classify: bool,
    pub pages: Option<PageSelection>,
}

/// Canonical `"start-end"` persisted form, matching the CLI/query syntax
/// `PageSelection::from_str` parses (see `docray-core::selection`).
fn page_selection_to_string(pages: PageSelection) -> String {
    format!("{}-{}", pages.start, pages.end)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

impl JobStore {
    pub fn new(db_path: &Path) -> JobStore {
        let conn = Connection::open(db_path).expect("cannot open job db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                error_code TEXT,
                error_message TEXT,
                input_path TEXT NOT NULL,
                granularity TEXT,
                format TEXT NOT NULL DEFAULT 'json',
                classify INTEGER NOT NULL DEFAULT 0,
                pages TEXT,
                result_path TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            -- Recover jobs orphaned by a previous crash/restart.
            UPDATE jobs SET status = 'queued' WHERE status = 'running';",
        )
        .expect("cannot init job db");
        let has_granularity = conn
            .prepare("PRAGMA table_info(jobs)")
            .expect("cannot inspect job schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("cannot read job schema")
            .any(|column| column.expect("cannot read job schema column") == "granularity");
        if !has_granularity {
            conn.execute("ALTER TABLE jobs ADD COLUMN granularity TEXT", [])
                .expect("cannot migrate job schema");
        }
        let has_format = conn
            .prepare("PRAGMA table_info(jobs)")
            .expect("cannot inspect job schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("cannot read job schema")
            .any(|column| column.expect("cannot read job schema column") == "format");
        if !has_format {
            conn.execute(
                "ALTER TABLE jobs ADD COLUMN format TEXT NOT NULL DEFAULT 'json'",
                [],
            )
            .expect("cannot migrate job schema");
        }
        let has_classify = conn
            .prepare("PRAGMA table_info(jobs)")
            .expect("cannot inspect job schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("cannot read job schema")
            .any(|column| column.expect("cannot read job schema column") == "classify");
        if !has_classify {
            conn.execute(
                "ALTER TABLE jobs ADD COLUMN classify INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .expect("cannot migrate job schema");
        }
        let has_pages = conn
            .prepare("PRAGMA table_info(jobs)")
            .expect("cannot inspect job schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("cannot read job schema")
            .any(|column| column.expect("cannot read job schema column") == "pages");
        if !has_pages {
            conn.execute("ALTER TABLE jobs ADD COLUMN pages TEXT", [])
                .expect("cannot migrate job schema");
        }
        JobStore {
            conn: Mutex::new(conn),
        }
    }

    // Poisoning is advisory here: the connection itself remains valid even if a
    // prior holder panicked mid-operation, and store methods return Result so
    // real (non-panic) SQLite errors are still surfaced to the caller.
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // Runtime store methods return Result so a transient SQLite error (e.g.
    // SQLITE_FULL) is surfaced to the caller instead of panicking and poisoning
    // the connection Mutex — which would brick every subsequent request.
    pub fn create(
        &self,
        id: &str,
        input_path: &str,
        granularity: Option<Granularity>,
        format: OutputFormat,
        classify: bool,
        pages: Option<PageSelection>,
    ) -> Result<(), rusqlite::Error> {
        let t = now();
        self.conn().execute(
            "INSERT INTO jobs (id, status, input_path, granularity, format, classify, pages, created_at, updated_at)
             VALUES (?1, 'queued', ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            rusqlite::params![
                id,
                input_path,
                granularity.map(Granularity::as_str),
                format.as_str(),
                classify,
                pages.map(page_selection_to_string),
                t
            ],
        )?;
        Ok(())
    }

    /// Atomically claim the oldest queued job. `Ok(None)` means the queue is
    /// empty; `Err` means the store failed (distinct so callers don't spin).
    pub fn claim_next(&self) -> Result<Option<ClaimedJob>, rusqlite::Error> {
        let conn = self.conn();
        #[allow(clippy::type_complexity)]
        let claimed: Option<(
            String,
            String,
            Option<String>,
            String,
            bool,
            Option<String>,
        )> = conn
            .query_row(
                "UPDATE jobs SET status = 'running', updated_at = ?1
             WHERE id = (SELECT id FROM jobs WHERE status = 'queued' ORDER BY created_at LIMIT 1)
             RETURNING id, input_path, granularity, format, classify, pages",
                rusqlite::params![now()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(
            claimed.map(|(id, input_path, level, format, classify, pages)| {
                let granularity = level.map(|value| {
                    value
                        .parse()
                        .expect("job granularity is validated before it reaches the store")
                });
                let format = format
                    .parse()
                    .expect("job format is validated before it reaches the store");
                let pages = pages.map(|value| {
                    PageSelection::from_str(&value)
                        .expect("job page selection is validated before it reaches the store")
                });
                ClaimedJob {
                    id,
                    input_path,
                    granularity,
                    format,
                    classify,
                    pages,
                }
            }),
        )
    }

    /// Queued plus running jobs: each one holds an upload on disk.
    pub fn count_pending(&self) -> Result<usize, rusqlite::Error> {
        self.conn().query_row(
            "SELECT COUNT(*) FROM jobs WHERE status IN ('queued','running')",
            [],
            |row| row.get(0),
        )
    }

    /// Records success for a running job. `Ok(false)` means the job is no
    /// longer running (expired by the sweeper meanwhile), so nothing now
    /// references `result_path` and the caller must delete it.
    pub fn mark_succeeded(&self, id: &str, result_path: &str) -> Result<bool, rusqlite::Error> {
        let updated = self.conn().execute(
            "UPDATE jobs SET status='succeeded', result_path=?2, updated_at=?3 WHERE id=?1 AND status='running'",
            rusqlite::params![id, result_path, now()],
        )?;
        Ok(updated == 1)
    }

    /// Records failure for a running job; `Ok(false)` if it is no longer
    /// running, in which case its existing terminal state is kept.
    pub fn mark_failed(
        &self,
        id: &str,
        code: &str,
        message: &str,
    ) -> Result<bool, rusqlite::Error> {
        let updated = self.conn().execute(
            "UPDATE jobs SET status='failed', error_code=?2, error_message=?3, updated_at=?4 WHERE id=?1 AND status='running'",
            rusqlite::params![id, code, message, now()],
        )?;
        Ok(updated == 1)
    }

    /// Fails jobs stuck non-terminal past the TTL (`expired`) and deletes
    /// their uploads, so a wedged queue or a stranded `running` row cannot pin
    /// its input forever. Running jobs are only considered stale after
    /// `min_running_secs` as well, which callers set beyond the extraction
    /// timeout so a live extraction is never expired. The rows themselves are
    /// removed by `sweep_expired` one TTL later, so clients can still read the
    /// `expired` outcome. Returns jobs expired.
    pub fn expire_stale(
        &self,
        ttl_secs: u64,
        min_running_secs: u64,
    ) -> Result<usize, rusqlite::Error> {
        let t = now();
        let queued_cutoff = t - ttl_secs as i64;
        let running_cutoff = t - ttl_secs.max(min_running_secs) as i64;
        let inputs: Vec<String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "UPDATE jobs SET status='failed', error_code='expired', error_message=?3, updated_at=?4
                 WHERE (status='queued' AND updated_at < ?1) OR (status='running' AND updated_at < ?2)
                 RETURNING input_path",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![
                    queued_cutoff,
                    running_cutoff,
                    "job did not finish within the retention period",
                    t
                ],
                |row| row.get(0),
            )?;
            rows.collect::<Result<_, _>>()?
        };
        // Lock released. A file that cannot be removed now is retried by
        // `sweep_expired` when the (now terminal) row itself expires.
        for input in &inputs {
            remove_ok(input);
        }
        Ok(inputs.len())
    }

    /// Deletes files in `uploads_dir` older than `min_age_secs` that no job
    /// row references. A server killed mid-upload leaves such files behind,
    /// and no row would ever sweep them. Uploads still being received have no
    /// row yet, so callers set `min_age_secs` beyond the upload deadline.
    pub fn sweep_orphan_uploads(
        &self,
        uploads_dir: &Path,
        min_age_secs: u64,
    ) -> std::io::Result<usize> {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(min_age_secs))
            .unwrap_or(std::time::UNIX_EPOCH);
        let mut removed = 0;
        for entry in std::fs::read_dir(uploads_dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if !meta.is_file() || meta.modified()? >= cutoff {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let referenced = self
                .conn()
                .query_row(
                    "SELECT 1 FROM jobs WHERE id=?1",
                    rusqlite::params![id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(std::io::Error::other)?
                .is_some();
            if !referenced && remove_ok(entry.path().to_str().unwrap_or_default()) {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// `Ok(None)` means no such job; `Err` means the store failed. Kept distinct
    /// so a DB error never masquerades as a 404.
    pub fn get(&self, id: &str) -> Result<Option<JobRow>, rusqlite::Error> {
        self.conn()
            .query_row(
                "SELECT id, status, error_code, error_message, result_path, format FROM jobs WHERE id=?1",
                rusqlite::params![id],
                |row| {
                    Ok(JobRow {
                        id: row.get(0)?,
                        status: row.get(1)?,
                        error_code: row.get(2)?,
                        error_message: row.get(3)?,
                        result_path: row.get(4)?,
                        format: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    /// Delete expired terminal rows and their files. Returns rows deleted.
    ///
    /// Ordering matters: we never hold the connection Mutex across filesystem
    /// ops, and we only delete a row once its files are gone (treating a missing
    /// file as already-cleaned) so we never orphan files on disk.
    pub fn sweep_expired(&self, ttl_secs: u64) -> Result<usize, rusqlite::Error> {
        let cutoff = now() - ttl_secs as i64;

        // (a) Under the lock, collect candidate ids + paths; then drop the lock.
        let candidates: Vec<(String, String, Option<String>)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT id, input_path, result_path FROM jobs \
                 WHERE updated_at < ?1 AND status IN ('succeeded','failed')",
            )?;
            let rows = stmt.query_map(rusqlite::params![cutoff], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        // (b) Delete files with the lock released. A NotFound file counts as
        // cleaned; any other error leaves the row in place for a later sweep.
        let cleaned: Vec<String> = candidates
            .into_iter()
            .filter(|(_, input, result)| {
                remove_ok(input) && result.as_deref().is_none_or(remove_ok)
            })
            .map(|(id, _, _)| id)
            .collect();

        // (c) Re-acquire the lock and delete only the fully-cleaned rows.
        let conn = self.conn();
        let mut deleted = 0usize;
        for id in &cleaned {
            deleted += conn.execute("DELETE FROM jobs WHERE id=?1", rusqlite::params![id])?;
        }
        Ok(deleted)
    }
}

/// Remove a file, treating an already-absent file as success.
pub fn remove_ok(path: &str) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn sweep_removes_only_expired_terminal_jobs() {
        let dir = std::env::temp_dir().join(format!("docray-jobs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        let input = dir.join("in.pdf");
        std::fs::write(&input, b"x").unwrap();
        store
            .create(
                "old",
                input.to_str().unwrap(),
                None,
                OutputFormat::Json,
                false,
                None,
            )
            .unwrap();
        assert_eq!(store.claim_next().unwrap().unwrap().id, "old");
        assert!(store.mark_failed("old", "crash", "boom").unwrap());
        store
            .create(
                "fresh",
                input.to_str().unwrap(),
                None,
                OutputFormat::Json,
                false,
                None,
            )
            .unwrap();

        // TTL 0 expires everything terminal that is at least 1s old; backdate 'old'.
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE jobs SET updated_at = updated_at - 100 WHERE id='old'",
                [],
            )
            .unwrap();
        }
        let swept = store.sweep_expired(50).unwrap();
        assert_eq!(swept, 1);
        assert!(store.get("old").unwrap().is_none());
        assert!(store.get("fresh").unwrap().is_some()); // queued jobs never swept
        std::fs::remove_dir_all(&dir).ok();
    }

    fn backdate(store: &JobStore, id: &str, secs: i64) {
        store
            .conn()
            .execute(
                "UPDATE jobs SET updated_at = updated_at - ?2 WHERE id=?1",
                rusqlite::params![id, secs],
            )
            .unwrap();
    }

    #[test]
    fn expire_stale_fails_stuck_jobs_and_deletes_their_uploads() {
        let dir = std::env::temp_dir().join(format!("docray-expire-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        let input = |id: &str| {
            let path = dir.join(id);
            std::fs::write(&path, b"x").unwrap();
            path.to_str().unwrap().to_string()
        };
        // Claimed in creation order: live-running, stuck-running, slow-running.
        for id in ["live-running", "stuck-running", "slow-running"] {
            store
                .create(id, &input(id), None, OutputFormat::Json, false, None)
                .unwrap();
            assert_eq!(store.claim_next().unwrap().unwrap().id, id);
        }
        for id in ["stuck-queued", "fresh-queued"] {
            store
                .create(id, &input(id), None, OutputFormat::Json, false, None)
                .unwrap();
        }
        // TTL 100s, running grace 1000s.
        backdate(&store, "stuck-queued", 200);
        backdate(&store, "stuck-running", 2000);
        // Past the TTL but inside the running grace: may still be extracting.
        backdate(&store, "slow-running", 200);

        assert_eq!(store.expire_stale(100, 1000).unwrap(), 2);
        for id in ["stuck-queued", "stuck-running"] {
            let job = store.get(id).unwrap().unwrap();
            assert_eq!(job.status, "failed", "{id}");
            assert_eq!(job.error_code.as_deref(), Some("expired"), "{id}");
            assert!(!dir.join(id).exists(), "{id} upload must be deleted");
        }
        assert_eq!(
            store.get("live-running").unwrap().unwrap().status,
            "running"
        );
        assert_eq!(
            store.get("slow-running").unwrap().unwrap().status,
            "running"
        );
        assert_eq!(store.get("fresh-queued").unwrap().unwrap().status, "queued");
        for id in ["live-running", "slow-running", "fresh-queued"] {
            assert!(dir.join(id).exists(), "{id} upload must be kept");
        }
        assert_eq!(store.count_pending().unwrap(), 3);

        // A worker finishing an expired job must not resurrect it; the caller
        // is told so it can delete the orphaned result.
        assert!(!store
            .mark_succeeded("stuck-running", "result.json")
            .unwrap());
        assert!(!store.mark_failed("stuck-running", "crash", "late").unwrap());
        let job = store.get("stuck-running").unwrap().unwrap();
        assert_eq!(job.error_code.as_deref(), Some("expired"));
        assert_eq!(job.result_path, None);
        assert!(store.mark_succeeded("live-running", "result.json").unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn orphan_upload_sweep_removes_only_old_unreferenced_files() {
        let dir = std::env::temp_dir().join(format!("docray-orphan-test-{}", std::process::id()));
        let uploads = dir.join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2000);
        for name in ["referenced", "orphan", "in-progress"] {
            let file = std::fs::File::create(uploads.join(name)).unwrap();
            if name != "in-progress" {
                file.set_modified(old).unwrap();
            }
        }
        let referenced = uploads.join("referenced");
        store
            .create(
                "referenced",
                referenced.to_str().unwrap(),
                None,
                OutputFormat::Json,
                false,
                None,
            )
            .unwrap();

        assert_eq!(store.sweep_orphan_uploads(&uploads, 1000).unwrap(), 1);
        assert!(!uploads.join("orphan").exists());
        assert!(referenced.exists(), "a job still references it");
        assert!(
            uploads.join("in-progress").exists(),
            "recent file may be an upload that has no row yet"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // Pins the atomicity of the UPDATE..RETURNING claim: many threads racing to
    // claim a small queue must each get a distinct job and none may claim twice.
    #[test]
    fn concurrent_claim_next_is_atomic() {
        let dir = std::env::temp_dir().join(format!("docray-claim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(JobStore::new(&dir.join("t.sqlite")));
        for i in 0..10 {
            store
                .create(
                    &format!("job-{i}"),
                    "in.pdf",
                    None,
                    OutputFormat::Json,
                    false,
                    None,
                )
                .unwrap();
        }

        let claimed = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let store = store.clone();
            let claimed = claimed.clone();
            handles.push(std::thread::spawn(move || {
                while let Some(job) = store.claim_next().unwrap() {
                    claimed.lock().unwrap().push(job.id);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mut ids = claimed.lock().unwrap().clone();
        assert_eq!(
            ids.len(),
            10,
            "every queued job must be claimed exactly once"
        );
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            10,
            "no job may be claimed by more than one thread"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claim_preserves_requested_output_options() {
        let dir = std::env::temp_dir().join(format!("dps-granularity-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        store
            .create(
                "word",
                "in.pdf",
                Some(Granularity::Word),
                OutputFormat::Lean,
                false,
                None,
            )
            .unwrap();

        let job = store.claim_next().unwrap().unwrap();
        assert_eq!(job.id, "word");
        assert_eq!(job.input_path, "in.pdf");
        assert_eq!(job.granularity, Some(Granularity::Word));
        assert_eq!(job.format, OutputFormat::Lean);
        assert!(!job.classify);
        assert_eq!(job.pages, None);

        store
            .create("classified", "in.pdf", None, OutputFormat::Json, true, None)
            .unwrap();
        let job = store.claim_next().unwrap().unwrap();
        assert_eq!(job.format, OutputFormat::Json);
        assert!(job.classify);
        std::fs::remove_dir_all(&dir).ok();
    }

    // Pins the "start-end" round-trip through the pages TEXT column: a page
    // selection persisted at `create` must come back byte-for-byte equivalent
    // (as a parsed `PageSelection`) from `claim_next`, and an absent selection
    // must round-trip as `None` (today's full-document job behavior).
    #[test]
    fn claim_preserves_page_selection() {
        let dir = std::env::temp_dir().join(format!("dps-pages-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        store
            .create(
                "ranged",
                "in.pdf",
                None,
                OutputFormat::Json,
                false,
                Some(PageSelection { start: 2, end: 3 }),
            )
            .unwrap();
        store
            .create("full", "in.pdf", None, OutputFormat::Json, false, None)
            .unwrap();

        let job = store.claim_next().unwrap().unwrap();
        assert_eq!(job.id, "ranged");
        assert_eq!(job.pages, Some(PageSelection { start: 2, end: 3 }));

        let job = store.claim_next().unwrap().unwrap();
        assert_eq!(job.id, "full");
        assert_eq!(job.pages, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
