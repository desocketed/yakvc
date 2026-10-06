//! `cargo xtask with-pipewire -- <command>`: runs a command against real
//! audio devices on a machine without a sound card.
//!
//! It starts a private PipeWire session with a test microphone and test
//! speakers, and points ALSA at it the way pipewire-alsa does on a desktop.
//! The engine then opens its devices through ALSA exactly as it does for
//! players, instead of using the dev test audio. A 440 Hz tone plays into the
//! test microphone. Needs Nix, which provides PipeWire, WirePlumber and D-Bus.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

/// A null source the tone is linked into, and a null sink for playback.
const DEVICES: &str = r#"
context.objects = [
    { factory = adapter
      args = { factory.name = support.null-audio-sink
               node.name = "test-mic"  node.description = "Test Microphone"
               media.class = "Audio/Source/Virtual"
               audio.position = [ MONO ]  audio.rate = 48000 } }
    { factory = adapter
      args = { factory.name = support.null-audio-sink
               node.name = "test-speakers"  node.description = "Test Speakers"
               media.class = "Audio/Sink"
               audio.position = [ FL FR ]  audio.rate = 48000 } }
]
"#;

/// The audio daemons, stopped when dropped.
struct Daemons(Vec<Child>);

impl Drop for Daemons {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Runs `command` inside a fresh PipeWire session and returns its exit code.
pub fn with_pipewire(command: &[String]) -> Result<i32> {
    let (program, args) = command.split_first().context("no command given")?;
    let pipewire = nix_package("pipewire")?;
    let wireplumber = nix_package("wireplumber")?;
    let dbus = nix_package("dbus")?;
    let alsa = nix_package("alsa-lib")?;

    let work = temp_dir("yakvc-audio")?;
    // Unix socket paths are limited to 108 bytes, so the runtime directory
    // must have a short path.
    let runtime = temp_dir("pw")?;
    let config = work.join("config");
    fs::create_dir_all(config.join("pipewire/pipewire.conf.d"))?;
    fs::write(
        config.join("pipewire/pipewire.conf.d/yakvc-test.conf"),
        DEVICES,
    )?;
    let asound = work.join("asound.conf");
    fs::write(
        &asound,
        format!(
            "<{alsa}/share/alsa/alsa.conf>\n\
             <{pw}/share/alsa/alsa.conf.d/50-pipewire.conf>\n\
             <{pw}/share/alsa/alsa.conf.d/99-pipewire-default.conf>\n",
            alsa = alsa.display(),
            pw = pipewire.display(),
        ),
    )?;
    let bus = format!("unix:path={}/bus", runtime.display());
    let env = [
        ("XDG_RUNTIME_DIR", runtime.clone().into_os_string()),
        ("XDG_CONFIG_HOME", config.into_os_string()),
        ("DBUS_SESSION_BUS_ADDRESS", bus.clone().into()),
        ("ALSA_CONFIG_PATH", asound.into_os_string()),
        (
            "ALSA_PLUGIN_DIR",
            pipewire.join("lib/alsa-lib").into_os_string(),
        ),
    ];
    let spawn = |program: PathBuf, args: &[&str], log: &str| -> Result<Child> {
        let log = fs::File::create(work.join(log))?;
        Command::new(&program)
            .args(args)
            .envs(env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .with_context(|| format!("starting {}", program.display()))
    };

    let mut daemons = Daemons(Vec::new());
    let session_conf = format!(
        "--config-file={}",
        dbus.join("share/dbus-1/session.conf").display()
    );
    let address = format!("--address={bus}");
    let dbus_args = [
        session_conf.as_str(),
        address.as_str(),
        "--nofork",
        "--nopidfile",
    ];
    daemons
        .0
        .push(spawn(dbus.join("bin/dbus-daemon"), &dbus_args, "dbus.log")?);
    sleep(Duration::from_secs(1));
    daemons
        .0
        .push(spawn(pipewire.join("bin/pipewire"), &[], "pipewire.log")?);
    sleep(Duration::from_secs(1));
    daemons.0.push(spawn(
        wireplumber.join("bin/wireplumber"),
        &[],
        "wireplumber.log",
    )?);

    let tone = [
        "--playback",
        "--raw",
        "--target",
        "test-mic",
        "--format",
        "s16",
        "--rate",
        "48000",
        "--channels",
        "1",
        "-",
    ];
    let mut player = spawn(pipewire.join("bin/pw-cat"), &tone, "pw-cat.log")?;
    let stdin = player.stdin.take().context("pw-cat has no stdin")?;
    daemons.0.push(player);
    std::thread::spawn(move || play_tone(stdin));
    // WirePlumber doesn't link a player into a virtual source, so the tone is
    // linked by hand once the player's ports exist.
    // The player's port is named after its channel, which depends on the
    // PipeWire version, so link whichever output it has.
    let pw_link = |args: &[&str]| {
        Command::new(pipewire.join("bin/pw-link"))
            .args(args)
            .envs(env.iter().cloned())
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let linked = (0..20).any(|_| {
        let port = pw_link(&["-o"]).and_then(|ports| {
            let port = ports.lines().find(|p| p.starts_with("pw-cat:"))?;
            Some(port.to_owned())
        });
        let ok = port.is_some_and(|port| pw_link(&[&port, "test-mic:input_MONO"]).is_some());
        if !ok {
            sleep(Duration::from_millis(500));
        }
        ok
    });
    ensure!(
        linked,
        "could not feed the test microphone (logs in {})",
        work.display()
    );
    eprintln!("PipeWire test devices ready (logs in {})", work.display());

    let status = Command::new(program)
        .args(args)
        .envs(env.iter().cloned())
        .status()
        .with_context(|| format!("running {program}"))?;
    Ok(status.code().unwrap_or(1))
}

/// Writes a 440 Hz sine at -10 dBFS, mono 16-bit 48 kHz, until the player
/// goes away.
fn play_tone(mut player: ChildStdin) {
    let mut i = 0u64;
    loop {
        let block: Vec<u8> = (0..4800)
            .flat_map(|_| {
                i += 1;
                let t = i as f32 / 48_000.0;
                let sample = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                ((sample * f32::from(i16::MAX)) as i16).to_le_bytes()
            })
            .collect();
        if player.write_all(&block).is_err() {
            return;
        }
    }
}

/// The store path of a nixpkgs package's main output.
fn nix_package(name: &str) -> Result<PathBuf> {
    let out = Command::new("nix")
        .args([
            "build",
            "--no-link",
            "--print-out-paths",
            &format!("nixpkgs#{name}^out"),
        ])
        .output()
        .context("running nix, which with-pipewire needs")?;
    if !out.status.success() {
        bail!(
            "nix build nixpkgs#{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)?;
    Ok(PathBuf::from(path.trim()))
}

fn temp_dir(prefix: &str) -> Result<PathBuf> {
    let out = Command::new("mktemp")
        .args(["-d", &format!("/tmp/{prefix}.XXXXXX")])
        .output()?;
    ensure!(out.status.success(), "mktemp failed");
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
}
