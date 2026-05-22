use crate::config::{DatabaseConfig, MetricConfig};
use crate::scorer::HealthScore;
use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::{Path, PathBuf};

const MIGRATIONS_SLICE: &[M<'_>] = &[M::up(
    "CREATE TABLE scores (
        commit_hash   TEXT NOT NULL,
        config_path   TEXT NOT NULL,
        timestamp     INTEGER NOT NULL,
        overall       REAL NOT NULL,
        health_score  TEXT NOT NULL,
        metric_config TEXT NOT NULL,
        created_at    INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
        PRIMARY KEY (commit_hash, config_path)
    );",
)];

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens (or creates) the SQLite file at `path`.
    /// Caller must only invoke after the user has approved creating a missing file.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        // WAL and busy-timeout are connection-level settings; apply before migrations.
        conn.pragma_update(None, "journal_mode", "WAL")
            .with_context(|| "Failed to set journal_mode=WAL")?;
        conn.pragma_update(None, "busy_timeout", 1000)
            .with_context(|| "Failed to set busy_timeout")?;

        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .with_context(|| "Failed to read journal_mode")?;
        if mode != "wal" {
            anyhow::bail!("SQLite journal_mode is '{mode}', expected 'wal'");
        }

        Migrations::from_slice(MIGRATIONS_SLICE)
            .to_latest(&mut conn)
            .with_context(|| "Failed to apply database migrations")?;

        Ok(Db { conn })
    }

    pub fn get_score(&self, sha: &str, config_path: &str) -> Result<Option<HealthScore>> {
        let mut stmt = self
            .conn
            .prepare("SELECT health_score FROM scores WHERE commit_hash = ?1 AND config_path = ?2")
            .with_context(|| "Failed to prepare get_score query")?;

        let mut rows = stmt.query([sha, config_path]).with_context(|| {
            format!("get_score query failed for {sha} with config {config_path}")
        })?;

        if let Some(row) = rows.next().with_context(|| "Error reading row")? {
            let json: String = row
                .get(0)
                .with_context(|| "Error reading health_score column")?;
            let score: HealthScore = serde_json::from_str(&json).with_context(|| {
                format!("Failed to deserialize HealthScore for {sha} with config {config_path}")
            })?;
            Ok(Some(score))
        } else {
            Ok(None)
        }
    }

    /// Persists the score. `timestamp` column is taken from `score.timestamp`
    /// (same instant as the JSON field), not the wall-clock insert time.
    pub fn upsert_score(
        &self,
        sha: &str,
        config_path: &str,
        score: &HealthScore,
        metrics: &[MetricConfig],
    ) -> Result<()> {
        let health_json =
            serde_json::to_string(score).with_context(|| "Failed to serialize HealthScore")?;
        let metrics_json =
            serde_json::to_string(metrics).with_context(|| "Failed to serialize MetricConfig")?;
        let ts = score.timestamp.timestamp();

        self.conn
            .execute(
                "INSERT INTO scores (commit_hash, config_path, timestamp, overall, health_score, metric_config)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(commit_hash, config_path) DO UPDATE SET
                     timestamp     = excluded.timestamp,
                     overall       = excluded.overall,
                     health_score  = excluded.health_score,
                     metric_config = excluded.metric_config",
                rusqlite::params![
                    sha,
                    config_path,
                    ts,
                    score.overall,
                    health_json,
                    metrics_json
                ],
            )
            .with_context(|| {
                format!("upsert_score failed for {sha} with config {config_path}")
            })?;
        Ok(())
    }
}

/// Resolve the database file path from config (defaults to `fiber.db` in CWD).
pub fn resolved_db_path(database: &DatabaseConfig) -> PathBuf {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    base.join(database.path.as_deref().unwrap_or("fiber.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricResult;
    use crate::scorer::{PenaltyNode, build_health_score};
    use chrono::Utc;
    use tempfile::NamedTempFile;

    const TEST_CONFIG_PATH: &str = "fiber.toml";

    #[test]
    fn migrations_are_valid() {
        assert!(Migrations::from_slice(MIGRATIONS_SLICE).validate().is_ok());
    }

    fn sample_score() -> HealthScore {
        let result = MetricResult {
            name: "test_metric".to_string(),
            total_penalty: 3.5,
            attributed: vec![("src/foo.ts".to_string(), 3.5)],
            unattributed: 0.0,
            details: "3 violations".to_string(),
        };
        build_health_score(
            vec![result],
            Some("abcdef1234567890".to_string()),
            Utc::now(),
        )
    }

    fn sample_metrics() -> Vec<MetricConfig> {
        vec![MetricConfig {
            name: "test_metric".to_string(),
            metric_type: "shell".to_string(),
            command: Some("echo 0".to_string()),
            error_penalty: None,
            warning_penalty: None,
            files: None,
            ast_count_type_reference: None,
            comment_startswith: None,
            comment_contains: None,
            max_function_lines: None,
            max_file_lines: None,
        }]
    }

    #[test]
    fn test_open_creates_table() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();
        // should not error; WAL mode check
        let mode: String = db
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_round_trip() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();
        let score = sample_score();
        let metrics = sample_metrics();
        let sha = score.commit.as_deref().unwrap();

        assert!(db.get_score(sha, TEST_CONFIG_PATH).unwrap().is_none());
        db.upsert_score(sha, TEST_CONFIG_PATH, &score, &metrics)
            .unwrap();
        assert!(db.get_score(sha, TEST_CONFIG_PATH).unwrap().is_some());

        let loaded = db.get_score(sha, TEST_CONFIG_PATH).unwrap().unwrap();
        assert_eq!(loaded.commit.as_deref(), Some(sha));
        assert_eq!(loaded.overall, score.overall);
        assert_eq!(loaded.commit, score.commit);
        // timestamp stored and round-tripped
        assert_eq!(loaded.timestamp.timestamp(), score.timestamp.timestamp());
    }

    #[test]
    fn test_upsert_overwrites() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();
        let mut score = sample_score();
        let metrics = sample_metrics();
        let sha = score.commit.clone().unwrap();

        db.upsert_score(&sha, TEST_CONFIG_PATH, &score, &metrics)
            .unwrap();
        // change overall and upsert again
        score.overall = 99.0;
        db.upsert_score(&sha, TEST_CONFIG_PATH, &score, &metrics)
            .unwrap();

        let loaded = db.get_score(&sha, TEST_CONFIG_PATH).unwrap().unwrap();
        assert_eq!(loaded.overall, 99.0);
    }

    #[test]
    fn test_penalty_node_round_trip() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();

        let result = MetricResult {
            name: "m".to_string(),
            total_penalty: 7.0,
            attributed: vec![("a/b.ts".to_string(), 7.0)],
            unattributed: 0.5,
            details: "detail".to_string(),
        };
        let score = build_health_score(
            vec![result],
            Some("deadbeef00000000".to_string()),
            Utc::now(),
        );
        let metrics = sample_metrics();
        let sha = score.commit.as_deref().unwrap();

        db.upsert_score(sha, TEST_CONFIG_PATH, &score, &metrics)
            .unwrap();
        let loaded = db.get_score(sha, TEST_CONFIG_PATH).unwrap().unwrap();

        // verify unattributed round-trips
        assert!((loaded.unattributed.get("m").copied().unwrap_or(0.0) - 0.5).abs() < 1e-9);
        // verify tree has the attributed file (the tree uses segment paths, so check for "a" dir and "b.ts" leaf)
        fn find_path(node: &PenaltyNode, target: &str) -> bool {
            if node.path == target {
                return true;
            }
            node.children.iter().any(|c| find_path(c, target))
        }
        assert!(find_path(&loaded.tree, "a"), "expected 'a' directory node");
        assert!(find_path(&loaded.tree, "b.ts"), "expected 'b.ts' leaf node");
    }

    #[test]
    fn test_timestamp_matches_score() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();
        let score = sample_score();
        let metrics = sample_metrics();
        let sha = score.commit.as_deref().unwrap();

        db.upsert_score(sha, TEST_CONFIG_PATH, &score, &metrics)
            .unwrap();

        // read the raw timestamp column
        let stored_ts: i64 = db
            .conn
            .query_row(
                "SELECT timestamp FROM scores WHERE commit_hash = ?1 AND config_path = ?2",
                [sha, TEST_CONFIG_PATH],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_ts, score.timestamp.timestamp());
    }

    #[test]
    fn test_different_config_paths_same_commit() {
        let tmp = NamedTempFile::new().unwrap();
        let db = Db::open(tmp.path()).unwrap();
        let sha = "abcdef1234567890";

        let mut score_a = sample_score();
        score_a.overall = 1.0;
        let mut score_b = sample_score();
        score_b.overall = 42.0;
        let metrics = sample_metrics();

        db.upsert_score(sha, "fiber.toml", &score_a, &metrics)
            .unwrap();
        db.upsert_score(sha, "configs/fiber.strict.toml", &score_b, &metrics)
            .unwrap();

        let loaded_a = db.get_score(sha, "fiber.toml").unwrap().unwrap();
        let loaded_b = db
            .get_score(sha, "configs/fiber.strict.toml")
            .unwrap()
            .unwrap();
        assert_eq!(loaded_a.overall, 1.0);
        assert_eq!(loaded_b.overall, 42.0);
    }

    #[test]
    fn test_prompt_create_db_file_non_terminal() {
        use crate::main_helpers::{CreateDbFile, prompt_create_database_file};
        use std::io::Cursor;
        use std::path::Path;

        let mut stdin = Cursor::new(b"c\n");
        let mut stdout = Vec::new();
        // is_terminal = false → should return No without reading stdin
        let result =
            prompt_create_database_file(Path::new("/tmp/test.db"), &mut stdin, &mut stdout, false)
                .unwrap();
        assert!(matches!(result, CreateDbFile::No));
        // stdin not consumed
        assert_eq!(stdin.position(), 0);
    }

    #[test]
    fn test_prompt_create_db_file_terminal_create() {
        use crate::main_helpers::{CreateDbFile, prompt_create_database_file};
        use std::io::Cursor;
        use std::path::Path;

        let mut stdin = Cursor::new(b"c\n");
        let mut stdout = Vec::new();
        let result =
            prompt_create_database_file(Path::new("/tmp/test.db"), &mut stdin, &mut stdout, true)
                .unwrap();
        assert!(matches!(result, CreateDbFile::Yes));
    }

    #[test]
    fn test_prompt_create_db_file_terminal_quit_default() {
        use crate::main_helpers::{CreateDbFile, prompt_create_database_file};
        use std::io::Cursor;
        use std::path::Path;

        // empty line → default quit
        let mut stdin = Cursor::new(b"\n");
        let mut stdout = Vec::new();
        let result =
            prompt_create_database_file(Path::new("/tmp/test.db"), &mut stdin, &mut stdout, true)
                .unwrap();
        assert!(matches!(result, CreateDbFile::No));
    }

    #[test]
    fn test_prompt_cached_action_non_terminal() {
        use crate::main_helpers::{CachedAction, prompt_cached_action};
        use std::io::Cursor;

        let mut stdin = Cursor::new(b"r\n");
        let mut stdout = Vec::new();
        // is_terminal = false → ShowCached immediately, no stdin read
        let result = prompt_cached_action(&mut stdin, &mut stdout, false).unwrap();
        assert!(matches!(result, CachedAction::ShowCached));
        assert_eq!(stdin.position(), 0);
    }

    #[test]
    fn test_prompt_cached_action_terminal_rerun() {
        use crate::main_helpers::{CachedAction, prompt_cached_action};
        use std::io::Cursor;

        let mut stdin = Cursor::new(b"r\n");
        let mut stdout = Vec::new();
        let result = prompt_cached_action(&mut stdin, &mut stdout, true).unwrap();
        assert!(matches!(result, CachedAction::ReRun));
    }

    #[test]
    fn test_prompt_cached_action_terminal_show_default() {
        use crate::main_helpers::{CachedAction, prompt_cached_action};
        use std::io::Cursor;

        // empty line → default ShowCached
        let mut stdin = Cursor::new(b"\n");
        let mut stdout = Vec::new();
        let result = prompt_cached_action(&mut stdin, &mut stdout, true).unwrap();
        assert!(matches!(result, CachedAction::ShowCached));
    }
}
