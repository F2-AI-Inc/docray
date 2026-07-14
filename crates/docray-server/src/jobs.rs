use docray_model::Granularity;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
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
    ) -> Result<(), rusqlite::Error> {
        let t = now();
        self.conn().execute(
            "INSERT INTO jobs (id, status, input_path, granularity, created_at, updated_at)
             VALUES (?1, 'queued', ?2, ?3, ?4, ?4)",
            rusqlite::params![id, input_path, granularity.map(Granularity::as_str), t],
        )?;
        Ok(())
    }

    /// Atomically claim the oldest queued job. `Ok(None)` means the queue is
    /// empty; `Err` means the store failed (distinct so callers don't spin).
    pub fn claim_next(
        &self,
    ) -> Result<Option<(String, String, Option<Granularity>)>, rusqlite::Error> {
        let conn = self.conn();
        let claimed: Option<(String, String, Option<String>)> = conn
            .query_row(
                "UPDATE jobs SET status = 'running', updated_at = ?1
             WHERE id = (SELECT id FROM jobs WHERE status = 'queued' ORDER BY created_at LIMIT 1)
             RETURNING id, input_path, granularity",
                rusqlite::params![now()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(claimed.map(|(id, input_path, level)| {
            let granularity = level.map(|value| {
                value
                    .parse()
                    .expect("job granularity is validated before it reaches the store")
            });
            (id, input_path, granularity)
        }))
    }

    pub fn mark_succeeded(&self, id: &str, result_path: &str) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "UPDATE jobs SET status='succeeded', result_path=?2, updated_at=?3 WHERE id=?1",
            rusqlite::params![id, result_path, now()],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, code: &str, message: &str) -> Result<(), rusqlite::Error> {
        self.conn().execute(
            "UPDATE jobs SET status='failed', error_code=?2, error_message=?3, updated_at=?4 WHERE id=?1",
            rusqlite::params![id, code, message, now()],
        )?;
        Ok(())
    }

    /// `Ok(None)` means no such job; `Err` means the store failed. Kept distinct
    /// so a DB error never masquerades as a 404.
    pub fn get(&self, id: &str) -> Result<Option<JobRow>, rusqlite::Error> {
        self.conn()
            .query_row(
                "SELECT id, status, error_code, error_message, result_path FROM jobs WHERE id=?1",
                rusqlite::params![id],
                |row| {
                    Ok(JobRow {
                        id: row.get(0)?,
                        status: row.get(1)?,
                        error_code: row.get(2)?,
                        error_message: row.get(3)?,
                        result_path: row.get(4)?,
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
fn remove_ok(path: &str) -> bool {
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
        store.create("old", input.to_str().unwrap(), None).unwrap();
        store.mark_failed("old", "crash", "boom").unwrap();
        store
            .create("fresh", input.to_str().unwrap(), None)
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

    // Pins the atomicity of the UPDATE..RETURNING claim: many threads racing to
    // claim a small queue must each get a distinct job and none may claim twice.
    #[test]
    fn concurrent_claim_next_is_atomic() {
        let dir = std::env::temp_dir().join(format!("docray-claim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(JobStore::new(&dir.join("t.sqlite")));
        for i in 0..10 {
            store.create(&format!("job-{i}"), "in.pdf", None).unwrap();
        }

        let claimed = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let store = store.clone();
            let claimed = claimed.clone();
            handles.push(std::thread::spawn(move || {
                while let Some((id, _, _)) = store.claim_next().unwrap() {
                    claimed.lock().unwrap().push(id);
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
    fn claim_preserves_requested_granularity() {
        let dir = std::env::temp_dir().join(format!("dps-granularity-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = JobStore::new(&dir.join("t.sqlite"));
        store
            .create("word", "in.pdf", Some(Granularity::Word))
            .unwrap();

        let (id, input_path, granularity) = store.claim_next().unwrap().unwrap();
        assert_eq!(id, "word");
        assert_eq!(input_path, "in.pdf");
        assert_eq!(granularity, Some(Granularity::Word));
        std::fs::remove_dir_all(&dir).ok();
    }
}
