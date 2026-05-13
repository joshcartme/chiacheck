use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask crate must live under workspace root")
        .map(Path::to_path_buf)
}
