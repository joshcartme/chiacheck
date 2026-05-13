use crate::config::DatabaseConfig;
use crate::db::{Db, resolved_db_path};
use anyhow::Result;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

pub enum CreateDbFile {
    Yes,
    No,
}

pub enum CachedAction {
    ShowCached,
    ReRun,
}

/// Prompts the user to create the missing database file.
///
/// When `is_terminal` is `false`, returns `No` immediately without reading stdin
/// (safe for scripts / CI).
pub fn prompt_create_database_file<R: BufRead, W: Write>(
    path: &Path,
    stdin: &mut R,
    stdout: &mut W,
    is_terminal: bool,
) -> Result<CreateDbFile> {
    if !is_terminal {
        return Ok(CreateDbFile::No);
    }

    write!(
        stdout,
        "Database file {} does not exist. (c)reate it / (q)uit [q]: ",
        path.display()
    )?;
    stdout.flush()?;

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();

    if trimmed == "c" || trimmed == "create" {
        Ok(CreateDbFile::Yes)
    } else {
        Ok(CreateDbFile::No)
    }
}

/// Prompts the user what to do when the current commit already has a cached score.
///
/// When `is_terminal` is `false`, returns `ShowCached` immediately without reading stdin.
pub fn prompt_cached_action<R: BufRead, W: Write>(
    sha: &str,
    stdin: &mut R,
    stdout: &mut W,
    is_terminal: bool,
) -> Result<CachedAction> {
    if !is_terminal {
        return Ok(CachedAction::ShowCached);
    }

    let short = &sha[..sha.len().min(12)];
    write!(
        stdout,
        "Commit {short}: cached score found. (s)how cached / (r)e-run [s]: "
    )?;
    stdout.flush()?;

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();

    if trimmed == "r" || trimmed == "re-run" || trimmed == "rerun" {
        Ok(CachedAction::ReRun)
    } else {
        Ok(CachedAction::ShowCached)
    }
}

pub const DECLINE_CREATE_DB_MSG: &str = "Fiber is configured to use a database but the file does not exist. \
     Set `enabled = false` under `[database]` or remove the `[database]` section \
     from your config if you do not want to use a database, then run again.";

/// Open the database if `database.enabled == true`. Handles the missing-file prompt.
/// Returns `None` when the database is disabled, `Some(Db)` when open, or an error
/// carrying [`DECLINE_CREATE_DB_MSG`] if the user declines to create the file.
///
/// Uses `std::io::stdin().is_terminal()` to decide whether to prompt when the file is
/// missing. That is often true under `cargo test` in an IDE terminal, so integration
/// tests should call [`open_db_if_enabled_interactive`] with `is_terminal: false`
/// instead.
pub fn open_db_if_enabled(database: &Option<DatabaseConfig>) -> Result<Option<Db>> {
    open_db_if_enabled_interactive(database, std::io::stdin().is_terminal())
}

/// Like [`open_db_if_enabled`], but the caller supplies whether stdin should be treated
/// as an interactive TTY for the missing-database prompt. When `is_terminal` is `false`,
/// the prompt is skipped and [`DECLINE_CREATE_DB_MSG`] is returned if the file is absent.
pub fn open_db_if_enabled_interactive(
    database: &Option<DatabaseConfig>,
    is_terminal: bool,
) -> Result<Option<Db>> {
    let cfg = match database {
        Some(d) if d.enabled => d,
        _ => return Ok(None),
    };

    let path = resolved_db_path(cfg);

    if !path.exists() {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        match prompt_create_database_file(&path, &mut stdin, &mut stdout, is_terminal)? {
            CreateDbFile::Yes => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            CreateDbFile::No => {
                anyhow::bail!("{DECLINE_CREATE_DB_MSG}");
            }
        }
    }

    Ok(Some(Db::open(&path)?))
}
