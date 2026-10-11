//! `cargo xtask pack`: launcher packs around the release jar, so a player
//! installs Yak VC by opening one file in the Modrinth App or Prism Launcher
//! instead of setting up Fabric and the mods folder by hand.
//!
//! Both packs carry the jar and Fabric API inside them (no downloads), so
//! they work offline and nothing depends on a mod site:
//! - `yakvc-<version>.mrpack`: the Modrinth modpack format, which the
//!   Modrinth App, Prism and ATLauncher import.
//! - `yakvc-<version>-prism.zip`: a MultiMC/Prism instance, which also sets
//!   the JVM flag the native library wants.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::{VERSION, root, run};

const FABRIC_MAVEN: &str = "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api";

pub(crate) fn pack(jar: &Path, out: &Path) -> Result<()> {
    ensure!(jar.is_file(), "no jar at {}", jar.display());
    let versions = Versions::read()?;
    let fabric_api = fabric_api(&versions.fabric_api)?;
    fs::create_dir_all(out)?;
    let mods = [jar, fabric_api.as_path()];

    let mrpack = out.join(format!("yakvc-{VERSION}.mrpack"));
    let mut zip = ZipWriter::new(File::create(&mrpack)?);
    let index = format!(
        r#"{{
  "formatVersion": 1,
  "game": "minecraft",
  "versionId": "{VERSION}",
  "name": "Yak VC",
  "summary": "Peer-to-peer proximity voice chat that needs nothing on the server.",
  "files": [],
  "dependencies": {{
    "minecraft": "{minecraft}",
    "fabric-loader": "{loader}"
  }}
}}
"#,
        minecraft = versions.minecraft,
        loader = versions.loader,
    );
    add_text(&mut zip, "modrinth.index.json", &index)?;
    for path in mods {
        add_file(
            &mut zip,
            &format!("overrides/mods/{}", file_name(path)?),
            path,
        )?;
    }
    zip.finish()?;

    let prism = out.join(format!("yakvc-{VERSION}-prism.zip"));
    let mut zip = ZipWriter::new(File::create(&prism)?);
    // Prism fills in the rest of the components (intermediary, LWJGL) from
    // these two when the instance first launches.
    let components = format!(
        r#"{{
  "formatVersion": 1,
  "components": [
    {{ "uid": "net.minecraft", "version": "{minecraft}", "important": true }},
    {{ "uid": "net.fabricmc.fabric-loader", "version": "{loader}" }}
  ]
}}
"#,
        minecraft = versions.minecraft,
        loader = versions.loader,
    );
    add_text(&mut zip, "mmc-pack.json", &components)?;
    add_text(
        &mut zip,
        "instance.cfg",
        "[General]\n\
         InstanceType=OneSix\n\
         name=Yak VC\n\
         iconKey=default\n\
         OverrideJavaArgs=true\n\
         JvmArgs=--enable-native-access=ALL-UNNAMED\n",
    )?;
    for path in mods {
        add_file(
            &mut zip,
            &format!("minecraft/mods/{}", file_name(path)?),
            path,
        )?;
    }
    zip.finish()?;

    println!("packed {}", mrpack.display());
    println!("packed {}", prism.display());
    Ok(())
}

/// The game, loader and Fabric API versions the mod is built against.
struct Versions {
    minecraft: String,
    loader: String,
    fabric_api: String,
}

impl Versions {
    fn read() -> Result<Self> {
        let path = root().join("mod/gradle.properties");
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let get = |key: &str| -> Result<String> {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .map(|value| value.trim().to_string())
                .with_context(|| format!("no {key} in {}", path.display()))
        };
        Ok(Self {
            minecraft: get("minecraft_version")?,
            loader: get("loader_version")?,
            fabric_api: get("fabric_api_version")?,
        })
    }
}

/// Fabric API's release jar from FabricMC's Maven, cached under target/.
fn fabric_api(version: &str) -> Result<PathBuf> {
    let dir = root().join("target/pack");
    fs::create_dir_all(&dir)?;
    let name = format!("fabric-api-{version}.jar");
    let path = dir.join(&name);
    if !path.exists() {
        let url = format!("{FABRIC_MAVEN}/{version}/{name}");
        run(Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&path)
            .arg(&url))?;
    }
    Ok(path)
}

fn file_name(path: &Path) -> Result<&str> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => Ok(name),
        None => bail!("bad file name {}", path.display()),
    }
}

fn add_text(zip: &mut ZipWriter<File>, name: &str, text: &str) -> Result<()> {
    zip.start_file(name, SimpleFileOptions::default())?;
    zip.write_all(text.as_bytes())?;
    Ok(())
}

fn add_file(zip: &mut ZipWriter<File>, name: &str, path: &Path) -> Result<()> {
    // Jars are already compressed.
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(name, options)?;
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    std::io::copy(&mut file, zip)?;
    Ok(())
}
