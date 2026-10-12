//! `cargo xtask two-clients`: the M4 exit check without sound hardware or a
//! display. Two dev clients on a local server hear each other, and a
//! spectator neither hears nor is heard.
//!
//! It starts a dev rendezvous, a local Fabric dedicated server in offline
//! mode (the mod is client-only, so the server is vanilla apart from the
//! loader) and two headless clients, Alice and Bob, with dev test audio (a
//! tone for the microphone, no speakers) and push-to-talk held. A client logs
//! `Talking <uuid> <name> true` (at DEBUG, so only in its `logs/debug.log`)
//! once it is actually playing that player's voice. It checks that:
//!   1. each client hears the other;
//!   2. after `/gamemode spectator Bob`, each stops hearing the other;
//!   3. after `/gamemode survival Bob`, each hears the other again.
//!
//! With `pipewire`, the clients use real devices instead: a private PipeWire
//! session each (`cargo xtask with-pipewire`), with a test microphone playing
//! a tone, reached through ALSA as on a desktop. It then also checks that
//! each microphone delivered audio and that no device failed to open.
//!
//! Running the Minecraft server means accepting the Minecraft EULA
//! (https://aka.ms/MinecraftEULA); `eula=true` is written into the throwaway
//! server directory only when the caller says so. Logs and game directories
//! are in `mod/build/two-clients/`.

use std::fs;
use std::io::Write;
use std::net::{TcpListener, UdpSocket};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::{dev, headless, root, run};

const PLAYERS: [&str; 2] = ["Alice", "Bob"];

pub(crate) fn two_clients(accept_eula: bool, pipewire: bool) -> Result<()> {
    if !accept_eula {
        bail!(
            "this runs a Minecraft server, which needs the Minecraft EULA accepted \
             (https://aka.ms/MinecraftEULA); pass --accept-eula or set YAKVC_ACCEPT_MINECRAFT_EULA=1 to accept it"
        );
    }
    let root = root();
    let work = root.join("mod/build/two-clients");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("server"))?;

    step("build the mod's launch arguments");
    run(Command::new(root.join("mod/gradlew"))
        .args(["writeDevLaunchArgs", "--console=plain", "--quiet"])
        .current_dir(root.join("mod")))?;
    let launch = root.join("mod/build/devlaunch");
    let display = headless::display()?;
    let render = headless::render_env()?;
    let mut parts = Parts(Vec::new());

    let rdv_port = free_port()?;
    step(&format!("start the dev rendezvous on 127.0.0.1:{rdv_port}"));
    let rendezvous = dev::prepare(rdv_port, &work.join("rdv"), false)?;
    parts.0.push(Part::new(
        Command::new(&rendezvous.server)
            .arg("run")
            .arg("--config")
            .arg(&rendezvous.config)
            .stdout(log(&work, "rdv.log")?)
            .stderr(log(&work, "rdv.log")?),
        false,
    )?);

    let mc_port = free_port()?;
    step(&format!(
        "start the Minecraft server on 127.0.0.1:{mc_port}"
    ));
    let server_dir = work.join("server");
    fs::write(server_dir.join("eula.txt"), "eula=true\n")?;
    fs::write(
        server_dir.join("server.properties"),
        format!(
            "online-mode=false\nenforce-secure-profile=false\nwhite-list=false\nenforce-whitelist=false\n\
             server-ip=127.0.0.1\nserver-port={mc_port}\nlevel-type=minecraft\\:flat\ndifficulty=peaceful\n\
             spawn-protection=0\nview-distance=4\nsimulation-distance=4\nmotd=Yak VC two-client check\n"
        ),
    )?;
    let mut server = Command::new("java")
        .args(args(&launch.join("server.args"))?)
        .arg("nogui")
        .current_dir(&server_dir)
        .stdin(Stdio::piped())
        .stdout(log(&work, "server.log")?)
        .stderr(log(&work, "server.log")?)
        .spawn()
        .context("starting the Minecraft server")?;
    let mut console = server.stdin.take().context("no server console")?;
    parts.0.push(Part {
        child: server,
        group: false,
    });

    wait_for(120, "the dev rendezvous", || {
        logged(&work.join("rdv.log"), 0, &["issuer id"])
    })?;
    wait_for(180, "the Minecraft server", || {
        logged(&work.join("server.log"), 0, &["Done ("])
    })?;

    for name in PLAYERS {
        let dir = work.join(name);
        fs::create_dir_all(dir.join("config/yakvc"))?;
        let mut config = rendezvous.client.clone();
        if !pipewire {
            config.push_str("\n[dev]\ntone_hz = 440.0\nnull_output = true\n");
        }
        fs::write(dir.join("config/yakvc/client.toml"), config)?;
        fs::write(
            dir.join("options.txt"),
            "onboardAccessibility:false\nskipMultiplayerWarning:true\njoinedFirstServer:true\nrenderDistance:2\nmaxFps:30\n",
        )?;
        step(&format!("start {name}"));
        // With PipeWire each client runs in its own session, in a process
        // group of its own, so stopping the group stops the daemons too.
        let mut cmd = if pipewire {
            let mut cmd = Command::new(std::env::current_exe()?);
            cmd.args(["with-pipewire", "--", "java"]);
            cmd
        } else {
            Command::new("java")
        };
        cmd.arg("-Dyakvc.dev.holdPushToTalk=true")
            .args(args(&launch.join("client.args"))?)
            .args(["--gameDir", ".", "--username", name])
            .args(["--quickPlayMultiplayer", &format!("127.0.0.1:{mc_port}")])
            .envs(render.iter().cloned())
            .env("DISPLAY", &display.name)
            .current_dir(&dir)
            .stdout(log(&work, &format!("{name}.log"))?)
            .stderr(log(&work, &format!("{name}.log"))?);
        parts.0.push(Part::new(&mut cmd, pipewire)?);
    }

    // Talking is logged at DEBUG, which only the dev log config's debug.log
    // keeps (along with every INFO line), so read that, not the console.
    let alice = work.join("Alice/logs/debug.log");
    let bob = work.join("Bob/logs/debug.log");
    let hears = |log: &Path, since: usize, who: &str| logged_talking(log, since, who, true);
    let stops_hearing =
        |log: &Path, since: usize, who: &str| logged_talking(log, since, who, false);

    step("1. each client hears the other");
    wait_for(300, "Alice to hear Bob", || hears(&alice, 0, "Bob"))?;
    wait_for(60, "Bob to hear Alice", || hears(&bob, 0, "Alice"))?;
    println!("ok");

    step("2. Bob becomes a spectator: neither hears the other");
    let (a, b) = (lines(&alice), lines(&bob));
    writeln!(console, "gamemode spectator Bob")?;
    wait_for(30, "Alice to stop hearing Bob", || {
        stops_hearing(&alice, a, "Bob")
    })?;
    wait_for(30, "Bob to stop hearing Alice", || {
        stops_hearing(&bob, b, "Alice")
    })?;
    let (a, b) = (lines(&alice), lines(&bob));
    sleep(Duration::from_secs(10));
    ensure!(
        !hears(&alice, a, "Bob") && !hears(&bob, b, "Alice"),
        "a spectator was heard, or heard someone (logs in {})",
        work.display()
    );
    println!("ok: nothing heard in 10 s");

    step("3. Bob leaves spectator mode: both hear each other again");
    let (a, b) = (lines(&alice), lines(&bob));
    writeln!(console, "gamemode survival Bob")?;
    wait_for(30, "Alice to hear Bob again", || hears(&alice, a, "Bob"))?;
    wait_for(30, "Bob to hear Alice again", || hears(&bob, b, "Alice"))?;
    println!("ok");

    if pipewire {
        step("real devices");
        for log in [&alice, &bob] {
            let text = fs::read_to_string(log)?;
            ensure!(
                text.contains("(yakvc) Microphone is delivering audio"),
                "no microphone audio in {}",
                log.display()
            );
            let failed = text.lines().find(|line| {
                line.contains("(yakvc) Engine: ")
                    && (line.contains("microphone") || line.contains("speakers"))
            });
            ensure!(
                failed.is_none(),
                "a device failed in {}: {}",
                log.display(),
                failed.unwrap_or("")
            );
        }
        println!("ok: both microphones delivered audio through PipeWire");
    }

    step("voice events");
    for (name, log) in [("Alice", &alice), ("Bob:  ", &bob)] {
        for line in fs::read_to_string(log)?.lines() {
            if ["Rendezvous", "Peer", "Talking"]
                .iter()
                .any(|what| line.contains(&format!("(yakvc) {what}")))
            {
                println!("{name}: {line}");
            }
        }
    }
    let _ = writeln!(console, "stop");
    step("passed");
    Ok(())
}

fn step(name: &str) {
    println!("\n==> {name}");
}

/// A started process, stopped when dropped.
struct Part {
    child: Child,
    /// It leads a process group of its own, which is stopped as a whole.
    group: bool,
}

impl Part {
    fn new(cmd: &mut Command, group: bool) -> Result<Part> {
        // The check runs on Linux only (Xvfb, PipeWire), but xtask also builds on Windows.
        #[cfg(unix)]
        if group {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("starting {:?}", cmd.get_program()))?;
        Ok(Part { child, group })
    }
}

/// Every part, stopped in reverse order when dropped, whether the check
/// passed or not.
struct Parts(Vec<Part>);

impl Drop for Parts {
    fn drop(&mut self) {
        for part in self.0.iter_mut().rev() {
            let target = if part.group {
                format!("-{}", part.child.id())
            } else {
                part.child.id().to_string()
            };
            let _ = Command::new("kill").args(["--", &target]).status();
            let _ = part.child.wait();
        }
    }
}

/// A log file opened for appending, shared by stdout and stderr.
fn log(work: &Path, name: &str) -> Result<fs::File> {
    Ok(fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(work.join(name))?)
}

/// The java command line Gradle wrote for a run config, one argument per line.
fn args(file: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    Ok(text.lines().map(str::to_owned).collect())
}

/// A port that nothing listens on, for TCP or UDP, as far as can be told.
fn free_port() -> Result<u16> {
    loop {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        if UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
}

/// Runs `done` every second until it holds, failing after `seconds`.
fn wait_for(seconds: u64, what: &str, mut done: impl FnMut() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for {what} (logs in mod/build/two-clients)"
        );
        sleep(Duration::from_secs(1));
    }
    Ok(())
}

/// The number of lines `log` has now, to look only at what follows.
fn lines(log: &Path) -> usize {
    fs::read_to_string(log).map_or(0, |text| text.lines().count())
}

/// Whether a line after the first `since` of `log` contains each of `parts`.
fn logged(log: &Path, since: usize, parts: &[&str]) -> bool {
    fs::read_to_string(log).is_ok_and(|text| {
        text.lines()
            .skip(since)
            .any(|line| parts.iter().all(|part| line.contains(part)))
    })
}

/// Whether `log` says, after line `since`, that `who` started (`on`) or
/// stopped talking: `(yakvc) Talking <uuid> <who> true`.
fn logged_talking(log: &Path, since: usize, who: &str, on: bool) -> bool {
    logged(log, since, &["(yakvc) Talking ", &format!(" {who} {on}")])
}
