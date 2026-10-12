//! Glue for `.github/workflows/ci.yml` that needs more than a one-line step.
//! Each reads what Actions provides through its usual environment variables.

use std::env;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

use crate::{output, root, run, sha256};

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

/// `cargo xtask release`, the `release` job on a version tag: the GitHub
/// release from the downloaded artifacts (`dist/`, `release/` with the CLIs,
/// `server/`), and the server image on GHCR.
pub(crate) fn release() -> Result<()> {
    let tag = env::var("GITHUB_REF_NAME")?;
    let version = tag.strip_prefix('v').context("not a version tag")?;
    let (dist, files, server) = (
        root().join("dist"),
        root().join("release"),
        root().join("server"),
    );
    let jar = dist.join(format!("yakvc-{version}.jar"));
    if !jar.exists() {
        let built: Vec<_> = fs::read_dir(&dist)?
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();
        bail!("tag {tag}, but built {}", built.join(" "));
    }

    for name in [
        format!("yakvc-{version}.jar"),
        format!("yakvc-{version}.mrpack"),
        format!("yakvc-{version}-prism.zip"),
    ] {
        fs::copy(dist.join(&name), files.join(&name)).with_context(|| format!("copying {name}"))?;
    }
    fs::copy(
        server.join("yakvc-server-linux-x86_64"),
        files.join("yakvc-server-linux-x86_64"),
    )?;
    let mut names: Vec<String> = fs::read_dir(&files)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    names.sort();
    // Checksums let a download by version (the NixOS server's, for one) pin
    // and check the file's hash.
    let mut sums = String::new();
    for name in &names {
        writeln!(sums, "{}  {name}", sha256(&files.join(name))?)?;
    }
    fs::write(files.join("SHA256SUMS"), sums)?;
    names.push("SHA256SUMS".into());

    // A version like 0.2.0-beta.1 makes a prerelease.
    let prerelease = version.contains('-');
    let mut gh = Command::new("gh");
    gh.args(["release", "create", &tag, "--generate-notes"]);
    if prerelease {
        gh.arg("--prerelease");
    }
    run(gh.args(names.iter().map(|name| files.join(name))))?;

    // Alphas are playtest builds: a GitHub prerelease with no server image.
    if version.contains("-alpha") {
        return Ok(());
    }
    let image = format!(
        "ghcr.io/{}/yakvc-server",
        env::var("GITHUB_REPOSITORY_OWNER")?
    );
    let mut login = Command::new("docker")
        .args(["login", "ghcr.io", "--password-stdin", "-u"])
        .arg(env::var("GITHUB_ACTOR")?)
        .stdin(Stdio::piped())
        .spawn()
        .context("running docker")?;
    login
        .stdin
        .take()
        .unwrap()
        .write_all(env::var("GH_TOKEN")?.as_bytes())?;
    ensure!(login.wait()?.success(), "docker login failed");
    run(Command::new("docker")
        .args(["load", "--input"])
        .arg(server.join("yakvc-server-image.tar.gz")))?;
    // Only a full release moves `latest`: a beta must not become what an
    // unpinned `docker pull` gets.
    let mut tags = vec![version];
    if !prerelease {
        tags.push("latest");
    }
    for tag in tags {
        let remote = format!("{image}:{tag}");
        run(Command::new("docker").args(["tag", "yakvc-server:latest", &remote]))?;
        run(Command::new("docker").args(["push", &remote]))?;
    }
    Ok(())
}
