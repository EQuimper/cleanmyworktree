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
    if fs::symlink_metadata(dst).is_ok() {
        return Err(CmwError::error(format!("destination already exists: {}", dst.display())));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match platform_clone(src, dst) {
        Ok(()) => Ok(CloneMethod::Clonefile),
        Err(e) if is_fallback_errno(&e) => {
            copy_recursive(src, dst)?;
            Ok(CloneMethod::Copy)
        }
        Err(e) => Err(CmwError::error(format!("clone {} -> {} failed: {e}", src.display(), dst.display()))),
    }
}

#[cfg(target_os = "macos")]
fn platform_clone(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    const CLONE_NOFOLLOW: u32 = 0x0001;
    let s = CString::new(src.as_os_str().as_bytes()).map_err(|_| std::io::Error::other("NUL in path"))?;
    let d = CString::new(dst.as_os_str().as_bytes()).map_err(|_| std::io::Error::other("NUL in path"))?;
    // SAFETY: both pointers are valid NUL-terminated C strings for the duration of the call.
    let rc = unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
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
        fs::set_permissions(dst, meta.permissions())?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
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
        assert!(fs::symlink_metadata(dst.join("a/link")).unwrap().file_type().is_symlink());

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
}
