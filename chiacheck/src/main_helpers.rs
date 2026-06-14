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

pub enum DirtyWorktreeStashChoice {
    Stash,
    Proceed,
}

/// When the working tree differs from `HEAD`, ask whether to stash before commands
/// that may check out other commits.
///
/// When `is_terminal` is `false`, returns [`DirtyWorktreeStashChoice::Proceed`] without
/// reading stdin (CI / scripts).
pub fn prompt_stash_dirty_worktree<R: BufRead, W: Write>(
    stdin: &mut R,
    stdout: &mut W,
    is_terminal: bool,
) -> Result<DirtyWorktreeStashChoice> {
    if !is_terminal {
        return Ok(DirtyWorktreeStashChoice::Proceed);
    }

    writeln!(
        stdout,
        "Working tree has uncommitted changes (per `git diff --quiet HEAD`)."
    )?;
    write!(
        stdout,
        "Stash them temporarily before running? (y)es / (n)o [n]: "
    )?;
    stdout.flush()?;

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();

    if trimmed == "y" || trimmed == "yes" {
        Ok(DirtyWorktreeStashChoice::Stash)
    } else {
        Ok(DirtyWorktreeStashChoice::Proceed)
    }
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

/// Prompts whether to read cached scores from the database or run metrics fresh.
///
/// When `is_terminal` is `false`, returns `ShowCached` immediately without reading stdin.
pub fn prompt_cached_action<R: BufRead, W: Write>(
    stdin: &mut R,
    stdout: &mut W,
    is_terminal: bool,
) -> Result<CachedAction> {
    if !is_terminal {
        return Ok(CachedAction::ShowCached);
    }

    write!(
        stdout,
        "Look up scores in the database, or run fresh? (u)se db / clean (r)un [u]: "
    )?;
    stdout.flush()?;

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();

    if trimmed == "r" || trimmed == "re-run" || trimmed == "rerun" || trimmed == "run" {
        Ok(CachedAction::ReRun)
    } else {
        Ok(CachedAction::ShowCached)
    }
}

pub const DECLINE_CREATE_DB_MSG: &str = "Chiacheck is configured to use a database but the file does not exist. \
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
    let is_terminal = std::io::stdin().is_terminal();
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    open_db_if_enabled_interactive(database, &mut stdin, &mut stdout, is_terminal)
}

/// Like [`open_db_if_enabled`], but the caller supplies IO handles and whether stdin
/// should be treated as an interactive TTY. When `is_terminal` is `false`, the prompt
/// is skipped and [`DECLINE_CREATE_DB_MSG`] is returned if the file is absent.
pub fn open_db_if_enabled_interactive<R: BufRead, W: Write>(
    database: &Option<DatabaseConfig>,
    stdin: &mut R,
    stdout: &mut W,
    is_terminal: bool,
) -> Result<Option<Db>> {
    let cfg = match database {
        Some(d) if d.enabled => d,
        _ => return Ok(None),
    };

    let path = resolved_db_path(cfg);

    if !path.exists() {
        match prompt_create_database_file(&path, stdin, stdout, is_terminal)? {
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

#[cfg(test)]
mod tests {
    use super::{DirtyWorktreeStashChoice, prompt_stash_dirty_worktree};
    use std::io::Cursor;

    #[test]
    fn prompt_stash_dirty_worktree_non_terminal_skips_read() {
        let mut stdin = Cursor::new(b"y\n");
        let mut stdout = Vec::new();
        let result = prompt_stash_dirty_worktree(&mut stdin, &mut stdout, false).unwrap();
        assert!(matches!(result, DirtyWorktreeStashChoice::Proceed));
        assert_eq!(stdin.position(), 0);
    }

    #[test]
    fn prompt_stash_dirty_worktree_terminal_yes() {
        let mut stdin = Cursor::new(b"y\n");
        let mut stdout = Vec::new();
        let result = prompt_stash_dirty_worktree(&mut stdin, &mut stdout, true).unwrap();
        assert!(matches!(result, DirtyWorktreeStashChoice::Stash));
    }

    #[test]
    fn prompt_stash_dirty_worktree_terminal_default_no() {
        let mut stdin = Cursor::new(b"\n");
        let mut stdout = Vec::new();
        let result = prompt_stash_dirty_worktree(&mut stdin, &mut stdout, true).unwrap();
        assert!(matches!(result, DirtyWorktreeStashChoice::Proceed));
    }
}
