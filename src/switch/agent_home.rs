//! Agent homes for `herdr switch` under `~/.herdr-switch/<agent>/`.
//!
//! Each entry links to an agent's real home, so `~/.herdr-switch/` shows which
//! homes switch launches use. Agents still run from their real homes.

use std::fs;
use std::io;
use std::os::unix::fs::{symlink, DirBuilderExt as _};
use std::path::{Path, PathBuf};

use tracing::warn;

/// Links `root/<agent>/<name>` to the agent's real home and returns the link.
/// A home that does not exist yet still gets its link; the agent creates it.
pub(super) fn link_agent_home(
    agent: &str,
    real_home: &Path,
    root: &Path,
) -> Result<PathBuf, String> {
    let target = real_home
        .canonicalize()
        .unwrap_or_else(|_| real_home.to_path_buf());
    let parent = root.join(agent);
    create_private_dir(&parent)?;
    let link = parent.join(home_dir_name(&target));
    ensure_link(&target, &link)?;
    Ok(link)
}

/// Readable, collision-free directory name for one real home.
fn home_dir_name(canonical_home: &Path) -> String {
    let hash = super::path_hash(canonical_home);
    match super::path_slug(canonical_home).trim_start_matches('.') {
        "" => hash,
        slug => format!("{slug}-{hash}"),
    }
}

pub(super) fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|err| format!("cannot create {}: {err}", path.display()))
}

/// Points `link` at `target`, leaving anything that is not a symlink in place.
pub(super) fn ensure_link(target: &Path, link: &Path) -> Result<(), String> {
    match fs::symlink_metadata(link) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if fs::read_link(link).ok().as_deref() == Some(target) {
                return Ok(());
            }
        }
        Ok(_) => {
            warn!(path = %link.display(), "keeping existing file in herdr switch home");
            return Ok(());
        }
        Err(err) => return Err(format!("cannot inspect {}: {err}", link.display())),
    }
    // A fresh link renamed into place never leaves the name missing.
    let staged = link.with_extension(format!("herdr-tmp-{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    symlink(target, &staged)
        .and_then(|()| fs::rename(&staged, link))
        .map_err(|err| format!("cannot link {}: {err}", link.display()))
}

#[cfg(test)]
#[path = "agent_home_tests.rs"]
mod tests;
