//! Glue for `.github/workflows/ci.yml` that needs more than a one-line step.
//! Each reads what Actions provides through its usual environment variables.

use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;

use anyhow::{Context, Result};

use crate::{output, root};

/// `cargo xtask changes`: whether this run builds, for the `changes` job.
pub(crate) fn changes() -> Result<()> {
    let code = must_build()?;
    let path = env::var("GITHUB_OUTPUT").context("GITHUB_OUTPUT is not set")?;
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    writeln!(file, "code={code}")?;
    Ok(())
}

/// Pull requests and merge queue entries build unless they only touch docs
/// (the files a push to main ignores). Pushes and manual runs always build,
/// except a push to main of a commit that already carries a version tag (the
/// tag's own run builds and releases it) or that the merge queue tested.
fn must_build() -> Result<bool> {
    let on_main = env::var("GITHUB_REF")? == "refs/heads/main";
    if on_main {
        let tags = git(&["tag", "--points-at", "HEAD"])?;
        if tags.lines().any(|tag| tag.starts_with('v')) {
            println!(
                "tagged {}: the tag run builds this commit",
                tags.replace('\n', " ")
            );
            return Ok(false);
        }
        if env::var("GITHUB_EVENT_NAME")? == "push" && queue_passed()? {
            println!("the merge queue already tested this commit");
            return Ok(false);
        }
    }
    // The pull request's or merge queue entry's base; empty for other events.
    let base = env::var("BASE").unwrap_or_default();
    if base.is_empty() {
        return Ok(true);
    }
    let changed = git(&["diff", "--name-only", &base, "HEAD"])?;
    Ok(changed.lines().any(|path| {
        !(path.ends_with(".md") || path.starts_with("docs/") || path.starts_with("LICENSE-"))
    }))
}

/// Whether a merge queue run's `ci-ok` passed for this very commit.
fn queue_passed() -> Result<bool> {
    let repo = env::var("GITHUB_REPOSITORY")?;
    let sha = env::var("GITHUB_SHA")?;
    let runs = output(Command::new("gh").args([
        "api",
        &format!("repos/{repo}/actions/runs?head_sha={sha}&event=merge_group"),
        "--jq",
        ".workflow_runs[].id",
    ]))?;
    for run in runs.lines() {
        let conclusions = output(Command::new("gh").args([
            "api",
            &format!("repos/{repo}/actions/runs/{run}/jobs"),
            "--jq",
            r#".jobs[] | select(.name == "ci-ok") | .conclusion"#,
        ]))?;
        if conclusions.lines().any(|c| c == "success") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git(args: &[&str]) -> Result<String> {
    output(Command::new("git").args(args).current_dir(root()))
}
