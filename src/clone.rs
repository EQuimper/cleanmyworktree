//! Copy-on-write cloning. On macOS/APFS a single `clonefile(2)` call clones a whole directory
//! tree (measured: 1.4 GB / 107k files in ~1.6 s, ~0 extra disk). Anything else falls back to a
//! regular recursive copy, and the caller is told so it can warn. Linux reflink slots in here later.

use crate::output::{CmwError, Result};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneMethod {
    /// Copy-on-write clone: shares blocks with the source until either side changes.
    Clonefile,
    /// Plain byte copy (different volume or filesystem without CoW). Costs full disk space.
    Copy,
}

/// Clone `src` (file or directory) to `dst`. `dst` must not exist; parent dirs are created.
pub fn clone_tree(src: &Path, dst: &Path) -> Result<CloneMethod> {
    clone_tree_inner(src, dst, false)
}

/// `force_copy` skips clonefile and goes straight to the fallback (used by tests).
fn clone_tree_inner(src: &Path, dst: &Path, force_copy: bool) -> Result<CloneMethod> {
    if fs::symlink_metadata(dst).is_ok() {
        return Err(CmwError::error(format!(
            "destination already exists: {}",
            dst.display()
        )));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let attempt = if force_copy {
        Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))
    } else {
        platform_clone(src, dst)
    };
    match attempt {
        Ok(()) => Ok(CloneMethod::Clonefile),
        Err(e) if is_fallback_errno(&e) => {
            copy_atomic(src, dst)?;
            Ok(CloneMethod::Copy)
        }
        Err(e) => {
            // clonefile is atomic, but never leave a destination that later reads as `exists`.
            remove_any(dst);
            Err(CmwError::error(format!(
                "clone {} -> {} failed: {e}",
                src.display(),
                dst.display()
            )))
        }
    }
}

fn remove_any(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// Copy into a temporary sibling, then rename onto `dst`, so an interrupted copy never leaves
/// a partial `dst` behind (the next hydrate would report it as `exists` forever).
fn copy_atomic(src: &Path, dst: &Path) -> Result<()> {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dst.with_file_name(format!(".{name}.cmw-partial-{}", std::process::id()));
    remove_any(&tmp);
    if let Err(e) =
        copy_recursive(src, &tmp).and_then(|()| fs::rename(&tmp, dst).map_err(Into::into))
    {
        remove_any(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform_clone(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    const CLONE_NOFOLLOW: u32 = 0x0001;
    let s = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in path"))?;
    let d = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("NUL in path"))?;
    // SAFETY: both pointers are valid NUL-terminated C strings for the duration of the call.
    let rc = unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_clone(_src: &Path, _dst: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))
}

fn is_fallback_errno(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(code) if code == libc::EXDEV || code == libc::ENOTSUP || code == libc::EOPNOTSUPP)
}

/// Recursive copy that preserves symlinks as symlinks and file permissions.
pub fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, dst)?;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        // Last, so a read-only source dir does not block writing its own children.
        fs::set_permissions(dst, meta.permissions())?;
        return Ok(());
    }
    fs::copy(src, dst)?;
    Ok(())
}

/// Copy a single file (used for .env and other secrets). Overwrites `dst`. On APFS std's copy
/// already clones; either way each worktree ends up with an independent file.
pub fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_directory_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("a/b/f.txt"), "hello").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("b/f.txt", src.join("a/link")).unwrap();

        let dst = tmp.path().join("out/dst");
        let method = clone_tree(&src, &dst).unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(method, CloneMethod::Clonefile);
        let _ = method;
        assert_eq!(fs::read_to_string(dst.join("a/b/f.txt")).unwrap(), "hello");
        assert!(
            fs::symlink_metadata(dst.join("a/link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );

        // Independent copies: writing to the clone leaves the source untouched.
        fs::write(dst.join("a/b/f.txt"), "changed").unwrap();
        assert_eq!(fs::read_to_string(src.join("a/b/f.txt")).unwrap(), "hello");
    }

    #[test]
    fn refuses_existing_destination() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("s")).unwrap();
        fs::create_dir(tmp.path().join("d")).unwrap();
        assert!(clone_tree(&tmp.path().join("s"), &tmp.path().join("d")).is_err());
    }

    #[test]
    fn copy_recursive_fallback_works() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("s");
        fs::create_dir_all(src.join("x")).unwrap();
        fs::write(src.join("x/y"), "1").unwrap();
        copy_recursive(&src, &tmp.path().join("d")).unwrap();
        assert_eq!(fs::read_to_string(tmp.path().join("d/x/y")).unwrap(), "1");
    }

    #[test]
    #[cfg(unix)]
    fn failed_fallback_copy_leaves_no_destination() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("s");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a"), "1").unwrap();
        fs::write(src.join("secret"), "2").unwrap();
        fs::set_permissions(src.join("secret"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(src.join("secret")).is_ok() {
            return; // running as root: chmod 000 does not block reads
        }
        let out = tmp.path().join("out");
        let dst = out.join("d");
        let res = clone_tree_inner(&src, &dst, true);
        fs::set_permissions(src.join("secret"), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(res.is_err());
        assert!(
            fs::symlink_metadata(&dst).is_err(),
            "partial destination left behind"
        );
        let leftovers: Vec<_> = fs::read_dir(&out).unwrap().flatten().collect();
        assert!(leftovers.is_empty(), "temp dir not cleaned: {leftovers:?}");

        // Retry after fixing the source succeeds and renames into place.
        assert_eq!(
            clone_tree_inner(&src, &dst, true).unwrap(),
            CloneMethod::Copy
        );
        assert_eq!(fs::read_to_string(dst.join("secret")).unwrap(), "2");
    }
}
