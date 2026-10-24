//! Session metadata binds a durable overlay WAL to source identity and geometry.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Metadata {
    source: String,
    sector_size: u32,
    length: u64,
    identity: String,
}
pub fn root() -> Result<PathBuf> {
    let root = std::env::var_os("RESC_SESSION_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("resc-sessions"));
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    Ok(fs::canonicalize(root)?)
}
pub fn open(
    source: &str,
    size: u32,
    length: u64,
    resume: Option<&str>,
) -> Result<(String, PathBuf, String)> {
    let identity = if source.starts_with(r"\\.\") {
        ensure!(
            resume.is_none(),
            "NotImplemented: resuming raw-device sessions requires immutable hardware identity"
        );
        format!("raw:{source}:{length}")
    } else {
        crate::safe_output::source_identity(&File::open(source)?)?
    };
    let expected = Metadata {
        source: source.into(),
        sector_size: size,
        length,
        identity: identity.clone(),
    };
    let id = match resume {
        Some(id) => uuid::Uuid::parse_str(id)?.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let directory = root()?.join(&id);
    if resume.is_some() {
        let path = directory.join("session.json");
        ensure!(
            fs::metadata(&path)?.len() < 65536,
            "Invalid session metadata size"
        );
        let previous: Metadata = serde_json::from_reader(File::open(path)?)?;
        ensure!(
            previous == expected,
            "Session source identity, length, or sector geometry changed"
        );
    } else {
        fs::create_dir(&directory)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("session.json"))?;
        file.write_all(&serde_json::to_vec(&expected)?)?;
        file.sync_all()?;
    }
    Ok((
        id,
        fs::canonicalize(&directory).context("Session directory missing")?,
        identity,
    ))
}
pub fn wal_path(directory: &Path) -> PathBuf {
    directory.join("overlay.wal")
}
