//! Synchronize ignored configuration files from the main checkout.
use crate::cli::EnvSyncArgs;
use crate::output::{CmwError, Ctx, Report, Result, sh_quote};
use crate::{clone, config, git, rules::Strategy};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct EnvSyncReport {
    target: PathBuf,
    source: PathBuf,
    dry_run: bool,
    files: Vec<EnvFile>,
}

#[derive(Debug, Serialize)]
struct EnvFile {
    path: String,
    status: &'static str,
}

impl Report for EnvSyncReport {
    fn human(&self) -> String {
        format!(
            "synced {} files to {}",
            self.files.len(),
            self.target.display()
        )
    }
}

pub fn sync(ctx: &Ctx, cwd: &Path, args: &EnvSyncArgs) -> Result<EnvSyncReport> {
    let target = git::toplevel(args.path.as_deref().unwrap_or(cwd))?;
    let source = git::main_worktree(cwd)?
        .ok_or_else(|| CmwError::not_found("no main checkout"))?
        .path;
    if std::fs::canonicalize(&target)? == std::fs::canonicalize(&source)? {
        return Err(CmwError::refused(
            "target is the main checkout",
            "cmw env sync <worktree-path>",
        ));
    }
    let (rules, _) = config::ruleset(&target)?;
    let mut files = Vec::new();
    let mut kept = false;
    for path in git::ignored_entries(&source)? {
        if path.ends_with('/')
            || !rules
                .classify(&path, false)
                .is_some_and(|r| r.strategy == Strategy::Copy)
        {
            continue;
        }
        let src = source.join(&path);
        if !src.is_file() {
            continue;
        }
        let dst = target.join(&path);
        let status = if std::fs::symlink_metadata(&dst).is_err() {
            if args.dry_run {
                "would_copy"
            } else {
                clone::copy_file(&src, &dst)?;
                "copied"
            }
        } else if std::fs::read(&src)? == std::fs::read(&dst)? {
            "unchanged"
        } else if args.force {
            if args.dry_run {
                "would_overwrite"
            } else {
                clone::copy_file(&src, &dst)?;
                "overwritten"
            }
        } else {
            kept = true;
            "kept"
        };
        files.push(EnvFile { path, status });
    }
    if kept {
        ctx.warn(format!(
            "some files were kept; run cmw env sync {} --force",
            sh_quote(&target.to_string_lossy())
        ));
    }
    Ok(EnvSyncReport {
        target,
        source,
        dry_run: args.dry_run,
        files,
    })
}
