//! `cargo xtask session`: a dev session on your own machine, in real windows
//! with real audio. A local rendezvous, an offline dev Minecraft server and
//! one client per name, each with its own game directory, joining the server
//! by themselves. Everything lives in `mod/run/dev/` and survives between
//! runs. It stops when any part exits, or on Ctrl-C.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use md5::Digest as _;
use serde_json::{Value, json};

use crate::{dev, root, run};

const RENDEZVOUS_PORT: u16 = 7843;
const SERVER_PORT: u16 = 25565;
const FLY_SPEED: f64 = 0.1;

/// A data pack that builds walls and rooms to listen through, in the flat
/// dev world (`xtask/rooms`).
const ROOMS: &[(&str, &str)] = &[
    ("pack.mcmeta", include_str!("../rooms/pack.mcmeta")),
    (
        "data/minecraft/tags/function/load.json",
        include_str!("../rooms/data/minecraft/tags/function/load.json"),
    ),
    (
        "data/yakvc/function/rooms.mcfunction",
        include_str!("../rooms/data/yakvc/function/rooms.mcfunction"),
    ),
    (
        "data/yakvc/function/build.mcfunction",
        include_str!("../rooms/data/yakvc/function/build.mcfunction"),
    ),
];

/// Grass on dirt on bedrock, the ground's top at y -61, as the rooms expect; the
/// void biome, so nothing ever spawns (an older world keeps plains, and the
/// build function clears its animals).
const FLAT_LAYERS: &str = r#"{"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:the_void"}"#;

/// Libraries Minecraft loads at runtime, from the flake's nixpkgs.
const LIBRARIES: &[&str] = &[
    "libglvnd",
    "vulkan-loader",
    "wayland",
    "libxkbcommon",
    "libpulseaudio",
    "alsa-lib",
    "xorg.libX11",
    "xorg.libXcursor",
    "xorg.libXrandr",
    "xorg.libXi",
    "xorg.libXext",
];

pub(crate) fn session(
    names: &[String],
    join: bool,
    talkers: &[String],
    talk_file: Option<&Path>,
) -> Result<()> {
    let root = root();
    let dir = root.join("mod/run/dev");
    run(Command::new(root.join("mod/gradlew"))
        .current_dir(root.join("mod"))
        .args(["writeDevLaunchArgs", "--quiet"]))?;
    let launch = root.join("mod/build/devlaunch");
    let library_path = library_path(&root)?;
    let talk_file = talk_file
        .map(|file| fs::canonicalize(file).with_context(|| file.display().to_string()))
        .transpose()?;

    let mut parts: Vec<(String, Child)> = Vec::new();
    let result = (|| {
        if !join {
            let rendezvous = dev::prepare(RENDEZVOUS_PORT, &dir.join("rendezvous"), false)?;
            let child = Command::new(&rendezvous.server)
                .arg("run")
                .arg("--config")
                .arg(&rendezvous.config)
                .spawn()
                .context("starting the rendezvous")?;
            parts.push(("rendezvous".into(), child));
            parts.push((
                "server".into(),
                server(&dir.join("server"), &launch, names)?,
            ));
        }
        // Quick play gives up if the server isn't listening yet.
        wait_for_server(&mut parts)?;
        let client_config = fs::read_to_string(dir.join("rendezvous/settings.json"))
            .context("no rendezvous yet: start a session without --join first")?;
        for name in names {
            let game_dir = dir.join(name);
            let talk = match (talkers.contains(name), &talk_file) {
                (false, _) => Talk::No,
                (true, None) => Talk::Beeps,
                (true, Some(file)) => Talk::File(file),
            };
            parts.push((
                name.clone(),
                client(
                    &game_dir,
                    name,
                    &client_config,
                    talk,
                    &launch,
                    &library_path,
                )?,
            ));
        }
        wait_any(&mut parts)
    })();
    for (_, child) in &mut parts {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// The dev Minecraft server in offline mode, so any name can join; the named
/// players are operators, so they (and the command file) can change the world.
fn server(dir: &Path, launch: &Path, names: &[String]) -> Result<Child> {
    fs::create_dir_all(dir)?;
    let ops: Vec<Value> = names
        .iter()
        .map(|name| json!({"uuid": offline_uuid(name), "name": name, "level": 4, "bypassesPlayerLimit": false}))
        .collect();
    fs::write(dir.join("ops.json"), dev::settings_json(&Value::Array(ops)))?;
    let eula = fs::read_to_string(dir.join("eula.txt")).unwrap_or_default();
    if !eula.lines().any(|line| line.trim() == "eula=true") {
        bail!(
            "read the Minecraft EULA (https://aka.ms/MinecraftEULA); to accept it, run\n  echo eula=true > {}",
            dir.join("eula.txt").display()
        );
    }
    write_once(
        &dir.join("server.properties"),
        &format!(
            "online-mode=false\nenforce-secure-profile=false\nwhite-list=false\nenforce-whitelist=false\n\
             server-ip=127.0.0.1\nserver-port={SERVER_PORT}\ndifficulty=peaceful\nspawn-protection=0\ngamemode=creative\nforce-gamemode=true\n\
             level-name=rooms\nlevel-type=minecraft\\:flat\ngenerator-settings={FLAT_LAYERS}\n"
        ),
    )?;
    // Written every time, so the rooms follow the repository.
    let pack = dir.join("rooms/datapacks/yakvc-rooms");
    for (path, text) in ROOMS {
        let path = pack.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, text)?;
    }
    Command::new("java")
        .args(launch_args(&launch.join("server.args"))?)
        .arg("nogui")
        .current_dir(dir)
        .spawn()
        .context("starting the Minecraft server")
}

/// One client in its own game directory, logged in as `name`, joining the server.
fn client(
    dir: &Path,
    name: &str,
    client_config: &str,
    talk: Talk,
    launch: &Path,
    library_path: &str,
) -> Result<Child> {
    // Written once, so each client's own settings survive between sessions.
    let settings = dir.join("config/yakvc/settings.json");
    write_once(&settings, client_config)?;
    set_talking(&settings, talk)?;
    write_once(
        &dir.join("options.txt"),
        "onboardAccessibility:false\nskipMultiplayerWarning:true\njoinedFirstServer:true\n",
    )?;
    // A nixpkgs Java can only load the host's GPU driver on NixOS; elsewhere
    // nixGL brings a matching one.
    let mut cmd = if Path::new("/etc/NIXOS").exists() {
        Command::new("java")
    } else {
        let mut nixgl = Command::new("nix");
        nixgl.args([
            "run",
            "--impure",
            "github:nix-community/nixGL",
            "--",
            "java",
        ]);
        nixgl
    };
    cmd.args(launch_args(&launch.join("client.args"))?)
        .args(["--gameDir", ".", "--username", name])
        .args([
            "--quickPlayMultiplayer",
            &format!("127.0.0.1:{SERVER_PORT}"),
        ])
        .env("LD_LIBRARY_PATH", library_path)
        .current_dir(dir)
        .spawn()
        .with_context(|| format!("starting {name}"))
}

/// The java command line Gradle wrote for a run config, one argument per line.
fn launch_args(file: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    Ok(text.lines().map(str::to_owned).collect())
}

/// The runtime libraries' directories, plus NixOS's GPU driver directory.
fn library_path(root: &Path) -> Result<String> {
    let out = Command::new("nix")
        .args(["build", "--no-link", "--print-out-paths", "--inputs-from"])
        .arg(root)
        .args(LIBRARIES.iter().map(|lib| format!("nixpkgs#{lib}")))
        .output()
        .context("running nix build")?;
    if !out.status.success() {
        bail!("nix build of the runtime libraries failed");
    }
    let mut dirs = vec!["/run/opengl-driver/lib".to_owned()];
    dirs.extend(
        String::from_utf8(out.stdout)?
            .lines()
            .map(|path| format!("{path}/lib")),
    );
    Ok(dirs.join(":"))
}

fn wait_for_server(parts: &mut [(String, Child)]) -> Result<()> {
    while TcpStream::connect(("127.0.0.1", SERVER_PORT)).is_err() {
        if let Some(name) = exited(parts)? {
            bail!("{name} exited before the server was up");
        }
        sleep(Duration::from_secs(1));
    }
    Ok(())
}

/// Returns once any part has exited, such as a client window being closed.
fn wait_any(parts: &mut [(String, Child)]) -> Result<()> {
    loop {
        if let Some(name) = exited(parts)? {
            println!("{name} exited; stopping the session");
            return Ok(());
        }
        sleep(Duration::from_millis(500));
    }
}

/// The name of a part that has exited, if any.
fn exited(parts: &mut [(String, Child)]) -> Result<Option<String>> {
    for (name, child) in parts {
        if child.try_wait()?.is_some() {
            return Ok(Some(name.clone()));
        }
    }
    Ok(None)
}

/// Whether a client talks on its own, and with what.
#[derive(Clone, Copy)]
enum Talk<'a> {
    No,
    /// The engine's built-in test input: beeps with quiet between them.
    Beeps,
    /// A WAV file, looping.
    File(&'a Path),
}

/// The engine's built-in beeps, as a microphone id (`TEST_INPUTS` in yakvc-audio).
const BEEPS: &str = "yakvc:test-beeps";

/// Makes the client talk on its own, or stop: a talker picks the beeps as its
/// microphone (or plays the file from `dev`) by voice activation, so nobody
/// has to hold push-to-talk for it. Rewritten every session; the rest of the
/// settings are left as the player set them.
fn set_talking(settings: &Path, talk: Talk) -> Result<()> {
    let mut json: Value = serde_json::from_str(&fs::read_to_string(settings)?)
        .with_context(|| format!("reading {}", settings.display()))?;
    let mut audio = json["audio"].take();
    if !audio.is_object() {
        audio = json!({});
    }
    match talk {
        // A talker's beeps go away; a player's own microphone stays.
        Talk::No => {
            if audio["input_device"] == BEEPS {
                audio.as_object_mut().unwrap().remove("input_device");
            }
        }
        Talk::Beeps => {
            audio["activation"] = json!("voice");
            audio["input_device"] = json!(BEEPS);
        }
        Talk::File(_) => {
            audio["activation"] = json!("voice");
            audio.as_object_mut().unwrap().remove("input_device");
        }
    }
    let top = json
        .as_object_mut()
        .context("settings.json must hold an object")?;
    top.insert("audio".to_owned(), audio);
    // The dev world is big, so creative flight goes twice as fast (vanilla is 0.05).
    let mut dev = json!({"fly_speed": FLY_SPEED});
    if let Talk::File(file) = talk {
        dev["wav"] = json!(file);
    }
    top.insert("dev".to_owned(), dev);
    fs::write(settings, dev::settings_json(&json))?;
    Ok(())
}

/// The UUID the offline-mode server gives `name` (as `yakvc_proto::offline_uuid`,
/// repeated here so xtask needn't build the networking crates).
fn offline_uuid(name: &str) -> uuid::Uuid {
    let hash = md5::Md5::new()
        .chain_update(b"OfflinePlayer:")
        .chain_update(name.as_bytes())
        .finalize();
    uuid::Builder::from_md5_bytes(hash.into()).into_uuid()
}

fn write_once(path: &Path, text: &str) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap_or(&PathBuf::from(".")))?;
        fs::write(path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talking(before: Value, talk: Talk) -> Value {
        let dir = std::env::temp_dir().join(format!("yakvc-talk-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let settings = dir.join(format!("settings-{}.json", before.to_string().len()));
        fs::write(&settings, dev::settings_json(&before)).unwrap();
        set_talking(&settings, talk).unwrap();
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap()
    }

    #[test]
    fn a_talker_gets_voice_activation_and_the_beeps() {
        let after = talking(
            json!({"dev_mode": true, "audio": {"activation": "push_to_talk", "input_device": "hw:1", "bitrate": 24000}}),
            Talk::Beeps,
        );
        assert_eq!(
            after,
            json!({"dev_mode": true, "audio": {"activation": "voice", "input_device": "yakvc:test-beeps", "bitrate": 24000}, "dev": {"fly_speed": 0.1}})
        );
        // Talking again changes nothing.
        assert_eq!(talking(after.clone(), Talk::Beeps), after);
        // Not talking puts the default microphone back, and keeps the activation.
        assert_eq!(
            talking(after, Talk::No),
            json!({"dev_mode": true, "audio": {"activation": "voice", "bitrate": 24000}, "dev": {"fly_speed": 0.1}})
        );
    }

    #[test]
    fn a_talker_with_a_file_plays_it_from_dev() {
        let after = talking(json!({"dev_mode": true}), Talk::File(Path::new("/b.wav")));
        assert_eq!(
            after,
            json!({"dev_mode": true, "audio": {"activation": "voice"}, "dev": {"fly_speed": 0.1, "wav": "/b.wav"}})
        );
        assert_eq!(
            talking(after, Talk::No),
            json!({"dev_mode": true, "audio": {"activation": "voice"}, "dev": {"fly_speed": 0.1}})
        );
    }

    #[test]
    fn a_player_s_own_microphone_is_kept_when_not_talking() {
        let before = json!({"audio": {"input_device": "hw:1"}});
        assert_eq!(
            talking(before.clone(), Talk::No),
            json!({"audio": {"input_device": "hw:1"}, "dev": {"fly_speed": 0.1}})
        );
    }
}
