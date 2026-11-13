//! Copy-on-write cloning.
//!
//! - macOS/APFS: a single `clonefile(2)` call clones a whole directory tree (measured: 1.4 GB /
//!   107k files in ~1.6 s, ~0 extra disk).
//! - Linux: the tree is walked and each file is cloned with `ioctl(FICLONE)` (reflink), which
//!   works on Btrfs, XFS (reflink=1), bcachefs and ZFS ≥ 2.2.
//!
//! Anything else (ext4, tmpfs, another volume) falls back to a regular recursive copy, and the
//! caller is told so it can warn.

use crate::output::{CmwError, Result};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneMethod {
    /// macOS `clonefile(2)`: the whole tree in one copy-on-write call.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Clonefile,
    /// Linux per-file reflink (`FICLONE`): every file shares blocks with the source.
    Reflink,
    /// Plain byte copy (different volume or filesystem without CoW). Costs full disk space.
    Copy,
}

/// Clone `src` (file or directory) to `dst`. `dst` must not exist; parent dirs are created.
pub fn clone_tree(src: &Path, dst: &Path) -> Result<CloneMethod> {
    clone_tree_inner(src, dst, false)
}

/// `force_copy` skips copy-on-write and goes straight to the plain copy (used by tests).
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
    #[cfg(target_os = "macos")]
    if !force_copy {
        match clonefile(src, dst) {
            Ok(()) => return Ok(CloneMethod::Clonefile),
            Err(e) if is_fallback_errno(&e) => {}
            Err(e) => {
                // clonefile is atomic, but never leave a destination that later reads as `exists`.
                let _ = remove_any(dst);
                return Err(CmwError::error(format!(
                    "clone {} -> {} failed: {e}",
                    src.display(),
                    dst.display()
                )));
            }
        }
    }
    let mut walk = Walk::new(cfg!(target_os = "linux") && !force_copy);
    copy_atomic(src, dst, &mut walk)?;
    Ok(walk.method())
}

/// Remove a file or a tree cmw created. A copied tree can contain read-only directories
/// (mode 0555 from the source), whose children can't be unlinked until the owner gets write
/// access back, so a failed first attempt restores it and retries.
fn remove_any(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path).or_else(|_| {
            make_owner_writable(path);
            fs::remove_dir_all(path)
        }),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Give the owner rwx on every directory under `path` (symlinks are not followed).
fn make_owner_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let mut perm = meta.permissions();
    perm.set_mode(perm.mode() | 0o700);
    let _ = fs::set_permissions(path, perm);
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            make_owner_writable(&entry.path());
        }
    }
}

/// Walk into a temporary sibling, then rename onto `dst`, so an interrupted walk never leaves
/// a partial `dst` behind (the next hydrate would report it as `exists` forever).
fn copy_atomic(src: &Path, dst: &Path, walk: &mut Walk) -> Result<()> {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dst.with_file_name(format!(".{name}.cmw-partial-{}", std::process::id()));
    let _ = remove_any(&tmp);
    if let Err(e) = walk
        .tree(src, &tmp)
        .and_then(|()| fs::rename(&tmp, dst).map_err(Into::into))
    {
        if let Err(rm) = remove_any(&tmp) {
            return Err(CmwError::error(format!(
                "{e}; also could not remove the partial copy {}: {rm}",
                tmp.display()
            ))
            .with_hint(format!(
                "rm -rf {}",
                crate::output::sh_quote(&tmp.to_string_lossy())
            )));
        }
        return Err(e);
    }
    Ok(())
}

/// Recursive tree copy. With `reflink` on, regular files are cloned with `FICLONE` until the
/// filesystem refuses once; every file after that is copied, and the walk reports `Copy`.
struct Walk {
    reflink: bool,
    fell_back: bool,
}

impl Walk {
    fn new(reflink: bool) -> Self {
        Self {
            reflink,
            fell_back: false,
        }
    }

    fn method(&self) -> CloneMethod {
        if self.reflink && !self.fell_back {
            CloneMethod::Reflink
        } else {
            CloneMethod::Copy
        }
    }

    /// Preserves symlinks as symlinks and file permissions.
    fn tree(&mut self, src: &Path, dst: &Path) -> Result<()> {
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
                self.tree(&entry.path(), &dst.join(entry.file_name()))?;
            }
            // Last, so a read-only source dir does not block writing its own children.
            fs::set_permissions(dst, meta.permissions())?;
            return Ok(());
        }
        if !meta.is_file() {
            // FIFOs, sockets and device nodes: opening a FIFO blocks until a writer shows up and a
            // socket can't be opened at all. They are runtime leftovers, never dependencies.
            return Ok(());
        }
        if self.reflink && !self.fell_back {
            match reflink_file(src, dst, &meta) {
                Ok(()) => return Ok(()),
                Err(e) if is_fallback_errno(&e) => self.fell_back = true,
                Err(e) => return Err(e.into()),
            }
        }
        fs::copy(src, dst)?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn clonefile(src: &Path, dst: &Path) -> std::io::Result<()> {
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

/// Clone one regular file with `ioctl(FICLONE)`. On failure `dst` is removed again, so the
/// caller can fall back to a plain copy.
#[cfg(target_os = "linux")]
fn reflink_file(src: &Path, dst: &Path, meta: &fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    let from = fs::File::open(src)?;
    let to = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(meta.permissions().mode())
        .open(dst)?;
    // SAFETY: both descriptors are open for the duration of the call; FICLONE takes the source
    // descriptor as its argument.
    let rc = unsafe { libc::ioctl(to.as_raw_fd(), libc::FICLONE, from.as_raw_fd()) };
    let res = if rc == 0 {
        // `mode` above is filtered by the umask; match the source exactly, like fs::copy does.
        to.set_permissions(meta.permissions())
    } else {
        Err(std::io::Error::last_os_error())
    };
    if res.is_err() {
        drop(to);
        let _ = fs::remove_file(dst);
    }
    res
}

#[cfg(not(target_os = "linux"))]
fn reflink_file(_src: &Path, _dst: &Path, _meta: &fs::Metadata) -> std::io::Result<()> {
    Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))
}

/// Errors that mean "this filesystem can't clone here", not "something is broken".
/// EINVAL and ENOTTY come from filesystems (or FUSE/overlay setups) without reflink support.
fn is_fallback_errno(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(code) if code == libc::EXDEV
        || code == libc::ENOTSUP
        || code == libc::EOPNOTSUPP
        || code == libc::EINVAL
        || code == libc::ENOTTY)
}

/// Recursive plain copy that preserves symlinks as symlinks and file permissions.
pub fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    Walk::new(false).tree(src, dst)
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
        // Set by the Linux CI/container runs that put TMPDIR on Btrfs or XFS.
        if std::env::var_os("CMW_TEST_EXPECT_REFLINK").is_some() {
            assert_eq!(method, CloneMethod::Reflink);
        }
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

    #[test]
    #[cfg(unix)]
    fn clone_keeps_file_modes() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("s");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("bin"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(src.join("bin"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(src.join("ro"), "x").unwrap();
        fs::set_permissions(src.join("ro"), fs::Permissions::from_mode(0o444)).unwrap();
        for force_copy in [false, true] {
            let dst = tmp.path().join(format!("d{force_copy}"));
            clone_tree_inner(&src, &dst, force_copy).unwrap();
            let mode = |p: &str| fs::metadata(dst.join(p)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode("bin"), 0o755);
            assert_eq!(mode("ro"), 0o444);
        }
    }

    #[test]
    fn walk_reports_copy_after_fallback() {
        let mut w = Walk::new(true);
        assert_eq!(w.method(), CloneMethod::Reflink);
        w.fell_back = true;
        assert_eq!(w.method(), CloneMethod::Copy);
        assert_eq!(Walk::new(false).method(), CloneMethod::Copy);
    }

    #[test]
    #[cfg(unix)]
    fn remove_any_handles_read_only_dirs() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("t");
        fs::create_dir_all(tree.join("ro/inner")).unwrap();
        fs::write(tree.join("ro/inner/f"), "x").unwrap();
        fs::set_permissions(tree.join("ro/inner"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(tree.join("ro"), fs::Permissions::from_mode(0o555)).unwrap();
        remove_any(&tree).unwrap();
        assert!(fs::symlink_metadata(&tree).is_err());
        remove_any(&tree).unwrap(); // already gone is fine
    }

    #[test]
    #[cfg(unix)]
    fn clone_skips_fifos() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("s");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("f"), "1").unwrap();
        let fifo = std::ffi::CString::new(src.join("pipe").to_str().unwrap()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
        for force_copy in [false, true] {
            let dst = tmp.path().join(format!("d{force_copy}"));
            clone_tree_inner(&src, &dst, force_copy).unwrap();
            assert_eq!(fs::read_to_string(dst.join("f")).unwrap(), "1");
            #[cfg(not(target_os = "macos"))]
            assert!(fs::symlink_metadata(dst.join("pipe")).is_err());
        }
    }
}
