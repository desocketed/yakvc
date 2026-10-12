//! Minecraft with no display or GPU: an Xvfb display and Mesa's software
//! renderers from the flake's nixpkgs. Minecraft 26.3 finds no sRGB GLX
//! config on Xvfb and falls back to Vulkan, which Mesa's lavapipe provides.
//!
//! The renderers come from the flake's locked nixpkgs (`--inputs-from`), not
//! the registry's channel, so they, and the guide screenshots they produce,
//! don't drift between runs.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail, ensure};

use crate::{root, run};

/// Libraries the game loads, besides Mesa.
const LIBRARIES: &[&str] = &[
    "libglvnd",
    "vulkan-loader",
    "xorg.libX11",
    "xorg.libXext",
    "xorg.libXcursor",
    "xorg.libXrandr",
    "xorg.libXxf86vm",
    "xorg.libXi",
    "xorg.libXrender",
    "xorg.libxcb",
    "alsa-lib",
    "libpulseaudio",
];

/// An Xvfb display, stopped when dropped.
pub(crate) struct Display {
    xvfb: Child,
    /// As `DISPLAY` takes it, such as `:93`.
    pub(crate) name: String,
}

impl Drop for Display {
    /// Stops Xvfb with SIGTERM, not `Child::kill`'s SIGKILL, so it removes its
    /// socket and lock file, which would otherwise look like a live display.
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg(self.xvfb.id().to_string())
            .status();
        let _ = self.xvfb.wait();
    }
}

/// Starts Xvfb on a free display, ready once its socket exists.
pub(crate) fn display() -> Result<Display> {
    let xvfb = store(&["xorg.xorgserver"])?.remove(0).join("bin/Xvfb");
    // Spread over a hundred displays, so parallel runs rarely meet; one that
    // is taken makes Xvfb exit at once, and the next is tried.
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .subsec_nanos() as usize;
    for attempt in 0..10 {
        let number = 90 + (seed + attempt * 37) % 100;
        // Taken, or left behind by a run that was killed.
        if Path::new(&format!("/tmp/.X{number}-lock")).exists() {
            continue;
        }
        let name = format!(":{number}");
        let mut child = Command::new(&xvfb)
            .args([&name, "-screen", "0", "1280x720x24"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting Xvfb")?;
        let socket = format!("/tmp/.X11-unix/X{number}");
        for _ in 0..100 {
            if Path::new(&socket).exists() {
                return Ok(Display { xvfb: child, name });
            }
            if child.try_wait()?.is_some() {
                break;
            }
            sleep(Duration::from_millis(100));
        }
        drop(Display { xvfb: child, name });
    }
    bail!("Xvfb found no free display")
}

/// The environment that points the game at Mesa's software renderers.
pub(crate) fn render_env() -> Result<Vec<(&'static str, String)>> {
    let mesa = store(&["mesa"])?.remove(0);
    let mut dirs = vec![mesa.join("lib").display().to_string()];
    dirs.extend(
        store(LIBRARIES)?
            .iter()
            .map(|lib| lib.join("lib").display().to_string()),
    );
    Ok(vec![
        ("LD_LIBRARY_PATH", dirs.join(":")),
        (
            "LIBGL_DRIVERS_PATH",
            mesa.join("lib/dri").display().to_string(),
        ),
        ("__GLX_VENDOR_LIBRARY_NAME", "mesa".to_owned()),
        (
            "VK_ICD_FILENAMES",
            mesa.join("share/vulkan/icd.d/lvp_icd.x86_64.json")
                .display()
                .to_string(),
        ),
    ])
}

/// The store paths of these nixpkgs packages, built if need be.
fn store(packages: &[&str]) -> Result<Vec<std::path::PathBuf>> {
    let out = Command::new("nix")
        .args(["build", "--no-link", "--print-out-paths", "--inputs-from"])
        .arg(root())
        .args(packages.iter().map(|package| format!("nixpkgs#{package}")))
        .output()
        .context("running nix build")?;
    ensure!(
        out.status.success(),
        "nix build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // A package with several outputs prints each; its first is the main one.
    let paths: Vec<_> = String::from_utf8(out.stdout)?
        .lines()
        .map(Into::into)
        .collect();
    ensure!(
        paths.len() >= packages.len(),
        "nix build printed too few paths"
    );
    Ok(paths)
}

/// `cargo xtask gametest`: the Fabric client gametest, headless. When it
/// passes, the user guide's screenshots in `docs/guide/` are refreshed from
/// it, those that look different (`scripts/UpdateGuideScreenshots.java`),
/// ready to commit. `gradle_args` go to Gradle.
pub(crate) fn gametest(gradle_args: &[String]) -> Result<()> {
    let root = root();
    let display = display()?;
    run(Command::new(root.join("mod/gradlew"))
        .args(["runClientGameTest", "--console=plain"])
        .args(gradle_args)
        .envs(render_env()?)
        .env("DISPLAY", &display.name)
        .current_dir(root.join("mod")))?;
    drop(display);
    run(Command::new("java")
        .arg("scripts/UpdateGuideScreenshots.java")
        .current_dir(&root))
}
