use rusqlite::{Connection, Result as SqlResult, params};
use sha2::{Sha256, Digest};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Helper: current timestamp as ISO-8601-like string (no chrono dependency)
// ---------------------------------------------------------------------------

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Decompose Unix seconds into (year, month, day, hour, min, sec).
    let mut days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Epoch starts at 1970-01-01.
    let mut year = 1970u32;
    loop {
        let days_in_year: u64 = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let month_days: [u64; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1;

    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ---------------------------------------------------------------------------
// Public data structs
// ---------------------------------------------------------------------------

/// Metadata about a code namespace in the constraint graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceInfo {
    pub name: String,
    pub head_hash: Option<String>,
    pub description: String,
}

/// A proposal to change a namespace (the Agora equivalent of a pull request).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub base_hash: Option<String>,
    pub namespace: String,
    pub status: String,
    /// The full Nous source that this proposal introduces.
    pub source: String,
    pub submitted_at: String,
    pub verified_at: Option<String>,
}

/// A single constraint stored in a namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintRecord {
    pub id: i64,
    pub namespace: String,
    pub constraint_text: String,
    pub kind: String,
    /// The proposal ID that added this constraint.
    pub added_by: String,
}

// ---------------------------------------------------------------------------
// ContentStore
// ---------------------------------------------------------------------------

/// Content-addressed storage for Nous source code, namespaces, proposals, and
/// constraints.  All content is keyed by its SHA-256 hash so that the same
/// bytes are never stored twice.
pub struct ContentStore {
    conn: Connection,
}

impl ContentStore {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Open (or create) the SQLite database at `path` and ensure all required
    /// tables exist.
    pub fn new(path: &str) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        // Enable WAL mode for better concurrent read performance.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let store = ContentStore { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Create all tables if they do not already exist.
    fn init_schema(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS blobs (
                hash        TEXT PRIMARY KEY,
                content     TEXT NOT NULL,
                created_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS namespaces (
                name        TEXT PRIMARY KEY,
                head_hash   TEXT,
                description TEXT
            );

            CREATE TABLE IF NOT EXISTS proposals (
                id           TEXT PRIMARY KEY,
                base_hash    TEXT,
                namespace    TEXT,
                status       TEXT,
                source       TEXT,
                submitted_at TEXT,
                verified_at  TEXT,
                merged_at    TEXT
            );

            CREATE TABLE IF NOT EXISTS constraints (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace         TEXT,
                constraint_text   TEXT,
                kind              TEXT,
                added_by_proposal TEXT
            );
            ",
        )
    }

    // -----------------------------------------------------------------------
    // Blob storage
    // -----------------------------------------------------------------------

    /// Hash `content` with SHA-256, persist it if not already present, and
    /// return the lowercase hex digest.
    pub fn store_blob(&self, content: &str) -> SqlResult<String> {
        let hash = sha256_hex(content);
        let now = now_iso();
        // INSERT OR IGNORE so re-storing the same content is a no-op.
        self.conn.execute(
            "INSERT OR IGNORE INTO blobs (hash, content, created_at) VALUES (?1, ?2, ?3)",
            params![hash, content, now],
        )?;
        Ok(hash)
    }

    /// Retrieve the content stored under `hash`, or `None` if not found.
    pub fn get_blob(&self, hash: &str) -> SqlResult<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT content FROM blobs WHERE hash = ?1",
        )?;
        let mut rows = stmt.query(params![hash])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    // -----------------------------------------------------------------------
    // Namespace management
    // -----------------------------------------------------------------------

    /// Create a new namespace.  Does nothing if the namespace already exists
    /// (INSERT OR IGNORE semantics).
    pub fn create_namespace(&self, name: &str, description: &str) -> SqlResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO namespaces (name, head_hash, description) VALUES (?1, NULL, ?2)",
            params![name, description],
        )?;
        Ok(())
    }

    /// Retrieve metadata for a single namespace, or `None` if it doesn't exist.
    pub fn get_namespace(&self, name: &str) -> SqlResult<Option<NamespaceInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, head_hash, description FROM namespaces WHERE name = ?1",
        )?;
        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(NamespaceInfo {
                name: row.get(0)?,
                head_hash: row.get(1)?,
                description: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Return all namespaces, ordered alphabetically.
    pub fn list_namespaces(&self) -> SqlResult<Vec<NamespaceInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, head_hash, description FROM namespaces ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NamespaceInfo {
                name: row.get(0)?,
                head_hash: row.get(1)?,
                description: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        })?;
        rows.collect()
    }

    // -----------------------------------------------------------------------
    // Proposal management
    // -----------------------------------------------------------------------

    /// Persist a proposal and return its ID.
    ///
    /// If the proposal has no ID, a content-derived ID is generated from the
    /// SHA-256 of the source combined with the submission timestamp so that
    /// identical sources submitted at different times yield distinct IDs.
    pub fn submit_proposal(&self, proposal: &Proposal) -> SqlResult<String> {
        let id = if proposal.id.is_empty() {
            let seed = format!("{}{}", proposal.source, proposal.submitted_at);
            sha256_hex(&seed)
        } else {
            proposal.id.clone()
        };
        let submitted_at = if proposal.submitted_at.is_empty() {
            now_iso()
        } else {
            proposal.submitted_at.clone()
        };

        self.conn.execute(
            "INSERT OR REPLACE INTO proposals
                (id, base_hash, namespace, status, source, submitted_at, verified_at, merged_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                id,
                proposal.base_hash,
                proposal.namespace,
                proposal.status,
                proposal.source,
                submitted_at,
                proposal.verified_at,
            ],
        )?;
        Ok(id)
    }

    /// Update the `status` field of a proposal.
    pub fn update_proposal_status(&self, id: &str, status: &str) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE proposals SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Retrieve a single proposal by ID.
    pub fn get_proposal(&self, id: &str) -> SqlResult<Option<Proposal>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, base_hash, namespace, status, source, submitted_at, verified_at
             FROM proposals WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Proposal {
                id: row.get(0)?,
                base_hash: row.get(1)?,
                namespace: row.get(2)?,
                status: row.get(3)?,
                source: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                submitted_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                verified_at: row.get(6)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all proposals for a given namespace, most recently submitted first.
    pub fn list_proposals(&self, namespace: &str) -> SqlResult<Vec<Proposal>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, base_hash, namespace, status, source, submitted_at, verified_at
             FROM proposals WHERE namespace = ?1 ORDER BY submitted_at DESC",
        )?;
        let rows = stmt.query_map(params![namespace], |row| {
            Ok(Proposal {
                id: row.get(0)?,
                base_hash: row.get(1)?,
                namespace: row.get(2)?,
                status: row.get(3)?,
                source: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                submitted_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                verified_at: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    // -----------------------------------------------------------------------
    // Constraint management
    // -----------------------------------------------------------------------

    /// Append a constraint to a namespace's constraint set.
    pub fn add_constraint(
        &self,
        namespace: &str,
        text: &str,
        kind: &str,
        proposal_id: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT INTO constraints (namespace, constraint_text, kind, added_by_proposal)
             VALUES (?1, ?2, ?3, ?4)",
            params![namespace, text, kind, proposal_id],
        )?;
        Ok(())
    }

    /// Return all constraints for a namespace, ordered by insertion order.
    pub fn get_constraints(&self, namespace: &str) -> SqlResult<Vec<ConstraintRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, namespace, constraint_text, kind, added_by_proposal
             FROM constraints WHERE namespace = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![namespace], |row| {
            Ok(ConstraintRecord {
                id: row.get(0)?,
                namespace: row.get(1)?,
                constraint_text: row.get(2)?,
                kind: row.get(3)?,
                added_by: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?;
        rows.collect()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute the SHA-256 digest of `input` and return it as a lowercase hex string.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
