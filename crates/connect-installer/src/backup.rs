//! Backups and safe writes of agent configuration files.

use crate::{InstallError, io_error};
use std::path::{Path, PathBuf};

/// Copy `path` to `<path>.watt-connect-<timestamp>.bak`. `None` if `path`
/// does not exist yet.
pub fn backup(path: &Path) -> Result<Option<PathBuf>, InstallError> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let file_name = path.file_name().map_or_else(
        || "config".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let target = path.with_file_name(format!("{file_name}.watt-connect-{stamp}.bak"));
    std::fs::copy(path, &target).map_err(io_error(path))?;
    restrict(&target);
    Ok(Some(target))
}

/// Write via a temp file and rename. Files we create are private (`0600`)
/// because they may contain merchant credentials; existing files keep their
/// permissions.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), InstallError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    let existed = path.exists();
    let permissions = std::fs::metadata(path).ok().map(|meta| meta.permissions());
    let tmp = path.with_extension("watt-connect.tmp");
    std::fs::write(&tmp, contents).map_err(io_error(&tmp))?;
    if let Some(permissions) = permissions {
        std::fs::set_permissions(&tmp, permissions).map_err(io_error(&tmp))?;
    } else if !existed {
        restrict(&tmp);
    }
    std::fs::rename(&tmp, path).map_err(io_error(path))
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_copies_and_write_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert!(backup(&path).unwrap().is_none());
        write_atomic(&path, b"{}").unwrap();
        let copy = backup(&path).unwrap().unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), b"{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
