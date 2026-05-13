use anyhow::Result;
use std::io::{BufRead, Write};
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
