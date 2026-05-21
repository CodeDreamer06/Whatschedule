use crate::model::{Job, JobStatus, Recipient, RecipientKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub id: i64,
    pub job_id: i64,
    pub recipient: Recipient,
    pub attempts: i64,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("could not open scheduler DB at {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        store.reset_inflight()?;
        Ok(store)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("scheduler db mutex poisoned");
        f(&conn)
    }

    fn migrate(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS contacts (
                    name TEXT PRIMARY KEY,
                    jid TEXT NOT NULL UNIQUE,
                    kind TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS jobs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message TEXT NOT NULL,
                    file_path TEXT,
                    run_at INTEGER NOT NULL,
                    recurrence_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS job_recipients (
                    job_id INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
                    jid TEXT NOT NULL,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    PRIMARY KEY (job_id, jid)
                );

                CREATE TABLE IF NOT EXISTS deliveries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    job_id INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
                    jid TEXT NOT NULL,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    status TEXT NOT NULL,
                    attempts INTEGER NOT NULL DEFAULT 0,
                    next_attempt_at INTEGER NOT NULL,
                    last_error TEXT,
                    sent_at INTEGER,
                    message_id TEXT
                );
                "#,
            )?;
            Ok(())
        })
    }

    fn reset_inflight(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE deliveries SET status = 'pending' WHERE status = 'sending'",
                [],
            )?;
            Ok(())
        })
    }

    pub fn insert_job(&self, job: &Job) -> Result<i64> {
        job.validate()?;
        self.with_conn(|conn| {
            let now = Local::now().timestamp();
            let recurrence = serde_json::to_string(&job.recurrence)?;
            conn.execute(
                "INSERT INTO jobs (message, file_path, run_at, recurrence_json, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    job.message,
                    job.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                    job.run_at.timestamp(),
                    recurrence,
                    job.status.to_string(),
                    now,
                ],
            )?;
            let job_id = conn.last_insert_rowid();
            for recipient in &job.recipients {
                insert_job_recipient(conn, job_id, recipient)?;
                insert_delivery(conn, job_id, recipient, job.run_at.timestamp())?;
            }
            Ok(job_id)
        })
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, message, file_path, run_at, recurrence_json, status FROM jobs ORDER BY run_at, id",
            )?;
            let rows = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let run_at_ts: i64 = row.get(3)?;
                let recurrence_json: String = row.get(4)?;
                let status: String = row.get(5)?;
                Ok((id, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, run_at_ts, recurrence_json, status))
            })?;

            let mut jobs = Vec::new();
            for row in rows {
                let (id, message, file_path, run_at_ts, recurrence_json, status) = row?;
                jobs.push(Job {
                    id: Some(id),
                    message,
                    file_path: file_path.map(PathBuf::from),
                    run_at: ts_to_local(run_at_ts),
                    recurrence: serde_json::from_str(&recurrence_json)?,
                    recipients: recipients_for(conn, id)?,
                    status: JobStatus::try_from(status.as_str())?,
                });
            }
            Ok(jobs)
        })
    }

    pub fn cancel_job(&self, id: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE jobs SET status = 'cancelled', updated_at = ?2 WHERE id = ?1",
                params![id, Local::now().timestamp()],
            )?;
            conn.execute(
                "UPDATE deliveries SET status = 'cancelled' WHERE job_id = ?1 AND status != 'sent'",
                params![id],
            )?;
            Ok(())
        })
    }

    pub fn due_deliveries(
        &self,
        now: DateTime<Local>,
        limit: usize,
    ) -> Result<Vec<(Job, Delivery)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT d.id, d.job_id, d.jid, d.name, d.kind, d.attempts
                 FROM deliveries d
                 JOIN jobs j ON j.id = d.job_id
                 WHERE d.status = 'pending'
                   AND d.next_attempt_at <= ?1
                   AND j.status = 'pending'
                   AND j.run_at <= ?1
                 ORDER BY d.next_attempt_at, d.id
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![now.timestamp(), limit as i64], |row| {
                Ok(Delivery {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    recipient: Recipient {
                        jid: row.get(2)?,
                        name: row.get(3)?,
                        kind: parse_kind(&row.get::<_, String>(4)?),
                    },
                    attempts: row.get(5)?,
                })
            })?;

            let mut out = Vec::new();
            for delivery in rows {
                let delivery = delivery?;
                if let Some(job) = job_by_id(conn, delivery.job_id)? {
                    out.push((job, delivery));
                }
            }
            Ok(out)
        })
    }

    pub fn mark_sending(&self, delivery_id: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE deliveries SET status = 'sending' WHERE id = ?1",
                params![delivery_id],
            )?;
            Ok(())
        })
    }

    pub fn mark_sent(&self, delivery_id: i64, message_id: &str) -> Result<()> {
        self.with_conn(|conn| {
            let now = Local::now().timestamp();
            conn.execute(
                "UPDATE deliveries SET status = 'sent', sent_at = ?2, message_id = ?3 WHERE id = ?1",
                params![delivery_id, now, message_id],
            )?;
            Ok(())
        })
    }

    pub fn mark_retry(&self, delivery_id: i64, attempts: i64, error: &str) -> Result<()> {
        self.with_conn(|conn| {
            let delay = retry_delay_seconds(attempts);
            let next = Local::now().timestamp() + delay;
            conn.execute(
                "UPDATE deliveries SET status = 'pending', attempts = ?2, next_attempt_at = ?3, last_error = ?4 WHERE id = ?1",
                params![delivery_id, attempts, next, error],
            )?;
            Ok(())
        })
    }

    pub fn complete_or_advance_job(&self, job_id: i64) -> Result<()> {
        self.with_conn(|conn| {
            let remaining: i64 = conn.query_row(
                "SELECT COUNT(*) FROM deliveries WHERE job_id = ?1 AND status NOT IN ('sent', 'cancelled')",
                params![job_id],
                |row| row.get(0),
            )?;
            if remaining > 0 {
                return Ok(());
            }

            let Some(job) = job_by_id(conn, job_id)? else {
                return Ok(());
            };
            if let Some(next) = job.recurrence.next_after(Local::now()) {
                conn.execute(
                    "UPDATE jobs SET run_at = ?2, status = 'pending', updated_at = ?3 WHERE id = ?1",
                    params![job_id, next.timestamp(), Local::now().timestamp()],
                )?;
                conn.execute("DELETE FROM deliveries WHERE job_id = ?1", params![job_id])?;
                for recipient in job.recipients {
                    insert_delivery(conn, job_id, &recipient, next.timestamp())?;
                }
            } else {
                conn.execute(
                    "UPDATE jobs SET status = 'sent', updated_at = ?2 WHERE id = ?1",
                    params![job_id, Local::now().timestamp()],
                )?;
            }
            Ok(())
        })
    }

    pub fn upsert_contact(&self, name: &str, recipient: &Recipient) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO contacts (name, jid, kind, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET jid = excluded.jid, kind = excluded.kind, updated_at = excluded.updated_at",
                params![name, recipient.jid, kind_to_str(&recipient.kind), Local::now().timestamp()],
            )?;
            Ok(())
        })
    }

    pub fn list_contacts(&self) -> Result<Vec<Recipient>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT name, jid, kind FROM contacts ORDER BY lower(name)")?;
            let rows = stmt.query_map([], |row| {
                Ok(Recipient {
                    name: row.get(0)?,
                    jid: row.get(1)?,
                    kind: parse_kind(&row.get::<_, String>(2)?),
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn remove_contact(&self, name: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM contacts WHERE name = ?1", params![name])?;
            Ok(())
        })
    }
}

fn job_by_id(conn: &Connection, id: i64) -> Result<Option<Job>> {
    let row = conn
        .query_row(
            "SELECT id, message, file_path, run_at, recurrence_json, status FROM jobs WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;

    Ok(match row {
        Some((id, message, file_path, run_at, recurrence_json, status)) => Some(Job {
            id: Some(id),
            message,
            file_path: file_path.map(PathBuf::from),
            run_at: ts_to_local(run_at),
            recurrence: serde_json::from_str(&recurrence_json)?,
            recipients: recipients_for(conn, id)?,
            status: JobStatus::try_from(status.as_str())?,
        }),
        None => None,
    })
}

fn recipients_for(conn: &Connection, job_id: i64) -> Result<Vec<Recipient>> {
    let mut stmt =
        conn.prepare("SELECT jid, name, kind FROM job_recipients WHERE job_id = ?1 ORDER BY name")?;
    let rows = stmt.query_map(params![job_id], |row| {
        Ok(Recipient {
            jid: row.get(0)?,
            name: row.get(1)?,
            kind: parse_kind(&row.get::<_, String>(2)?),
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn insert_job_recipient(conn: &Connection, job_id: i64, recipient: &Recipient) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO job_recipients (job_id, jid, name, kind) VALUES (?1, ?2, ?3, ?4)",
        params![
            job_id,
            recipient.jid,
            recipient.name,
            kind_to_str(&recipient.kind)
        ],
    )?;
    Ok(())
}

fn insert_delivery(
    conn: &Connection,
    job_id: i64,
    recipient: &Recipient,
    next_attempt_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO deliveries (job_id, jid, name, kind, status, attempts, next_attempt_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5)",
        params![
            job_id,
            recipient.jid,
            recipient.name,
            kind_to_str(&recipient.kind),
            next_attempt_at
        ],
    )?;
    Ok(())
}

fn parse_kind(value: &str) -> RecipientKind {
    if value == "group" {
        RecipientKind::Group
    } else {
        RecipientKind::Contact
    }
}

fn kind_to_str(kind: &RecipientKind) -> &'static str {
    match kind {
        RecipientKind::Contact => "contact",
        RecipientKind::Group => "group",
    }
}

fn ts_to_local(ts: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Local::now)
}

fn retry_delay_seconds(attempts: i64) -> i64 {
    let capped = attempts.clamp(1, 8) as u32;
    let base = 30_i64.saturating_mul(2_i64.saturating_pow(capped - 1));
    let jitter = rand::random_range(0..=15);
    base + jitter
}
