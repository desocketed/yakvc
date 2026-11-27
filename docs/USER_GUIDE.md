# Yak VC — User guide

Yak VC is proximity voice chat for Minecraft. You hear players near you, from the direction they are standing, and quieter the further away they are. It is a client-only Fabric mod: servers need no plugin, but everyone who wants to talk needs the mod. The mod has no screens of its own: you talk with a key, and everything else is set in the [settings file](#the-settings-file).

## Installing

The easiest way is a launcher that installs everything for you. Get the [Modrinth App](https://modrinth.com/app) or [Prism Launcher](https://prismlauncher.org/), then download one file from the [latest release](https://github.com/desocketed/yakvc/releases):

- **Modrinth App:** download `yakvc-<version>.mrpack` and open it (or drag it onto the app). It makes a new profile with Minecraft, Fabric and Yak VC; press Play.
- **Prism Launcher:** download `yakvc-<version>-prism.zip`, then **Add Instance → Import** and pick the file. The instance already has the Java setting below.

To update, import the newer file the same way; your voice settings live in the instance's `config/yakvc` folder, so copy it across if you want to keep them.

### By hand, or with the official launcher

1. Install [Fabric Loader](https://fabricmc.net/use/) for Minecraft 26.3, and put [Fabric API](https://modrinth.com/mod/fabric-api) in your `mods` folder.
2. Put the Yak VC jar in the same `mods` folder.
3. Add this to your launcher's JVM arguments (in the official launcher: Installations → Edit → More Options → JVM Arguments):

   ```
   --enable-native-access=ALL-UNNAMED
   ```

   Yak VC's voice engine is a native library, loaded through Java's Foreign Function & Memory API. Without the flag Java 25 prints a warning at startup, and a future Java release will refuse to load the library.

Yak VC runs on Windows (x86_64), macOS (Intel and Apple silicon) and Linux (x86_64 and ARM64).

When the game starts, Yak VC signs in to the Yak VC voice server with your Minecraft account, the same way the game does when you join a server. Your password and access token go only to Mojang, never to Yak VC.

You don't need a Minecraft account to talk on offline-mode servers (servers that don't check accounts). Without one, Yak VC signs you in *unverified*: you talk as usual there, but on servers that check accounts you get no voice, and players who turned on `verified_only` don't hear you. Players with a verified account show a badge (a cyan check mark), and when a verified and an unverified player claim the same identity, the verified one wins.

## Keys

The keys are in Options → Controls → Key Binds, under **Yak VC**.

| Key | Default | What it does |
| --- | --- | --- |
| Push to Talk | V | Hold to talk. |
| Toggle Mute | not bound | Turns your microphone off and on. |
| Toggle Deafen | not bound | Stops you hearing anyone. Your microphone still works unless you mute it too. |
| Toggle Debug Overlay | not bound | Shows or hides the [debug overlay](#the-debug-overlay). |

Mute, deafen and the debug overlay are unbound so they don't clash with other mods.

## Who is talking

A green speaker floats over the head of every player who is talking, above their name tag, with the verified badge next to it when their account is verified. It shows through walls, like name tags do. In third person you see it over your own head too.


## Using another voice server

Yak VC's own server matches players who use it, but a group of friends can run their own `yakvc-server` instead, for example to keep their play entirely among themselves. The [deployment guide](../deploy/README.md) says how. Once it runs, `yakvc-server code --config /etc/yakvc/config.toml` prints the server's code (and the server prints it at each start), one line like

```
yakvc1:5c9578695b7b…2cde60:7e2624000ff2…bd20f:https://relay.example.com/
```

which names the server, its ticket signing key and its relay. Everyone who should meet on that server puts the code in the `server` key of the [settings file](#the-settings-file) and restarts the game; a code belongs to one server, so players on different codes don't hear each other. The same code can go in a modpack's [preset](#modpack-presets).


## Privacy

- **Other players learn your IP address.** Voice goes straight from player to player, and Yak VC connects to every Yak VC player on the server as soon as you both join, so all of them can see your IP address, not just the ones near you. On a server network with one shared player list, that is every Yak VC player on the network. Set `relay_only` to hide it: everything then goes through the Yak VC relay. A relay has limited room, so with `relay_only` about 11 nearby players can hear you at once; the nearest get it first.
- **You talk only to players you can see nearby.** Yak VC sends your voice only to players your game shows within range, and plays only theirs. Spectators neither hear nor are heard. Voice is encrypted from player to player; the relay only passes on encrypted data.
- **The Yak VC server** learns which Yak VC players are on a server together and their IP addresses, but never hears what you say, and it keeps no record of who played with whom. Your game sends it the rest of the player list only as scrambled codes, but player IDs are public, so it could work out who is on your list.  The same goes for anyone else's server you paste as the `server` key, so only use codes from people you trust.
- **Chat restrictions.** If your Microsoft account (for example a child account) or your launcher turns chat off, Yak VC turns voice off too. `respect_chat_restrictions` controls this.
- **Blocked players.** Anyone you block in Social Interactions is muted both ways. `mute_blocked_players` controls this.
- **Server owners** can ask Yak VC to stay off: put `[no-yakvc]` anywhere in the server's MOTD, and Yak VC turns itself off for everyone on that server. Nothing enforces this; the mod simply honours it.

## When something goes wrong

Yak VC shows a message in the top-right corner when voice, or a part of it, stops working:

| Message | What to do |
| --- | --- |
| Voice chat doesn't support this system yet. | Your system isn't one of those listed under [Installing](#installing). |
| Voice chat couldn't load its engine. See the game log. | Download the Yak VC jar again and replace the old one. Antivirus software can also block the library Yak VC unpacks into `config/yakvc/natives/`. |
| Voice chat couldn't start: … | Usually a mistake in `config/yakvc/settings.json`; the message says which. Fix it, or delete the file to get the defaults back. |
| Voice chat crashed and is off until you restart the game. | Restart the game. If it keeps happening, report it with your game log. |
| Cannot open the microphone: … or The microphone stopped working: … | Check that the microphone is plugged in, not used by another program, and allowed for Java in your system's privacy settings, or pick another one with `audio.input_device` in the settings file. Yak VC keeps trying. |
| Cannot open the speakers: … or The speakers stopped working: … | The same for your speakers or headphones, and `audio.output_device`. You can still talk. |
| Voice chat sign-in failed: … | The Yak VC server didn't accept your Minecraft account; the message says why. Yak VC tries again by itself. |

A message that keeps coming back, such as a sign-in that fails at every try, shows at most once every five minutes.

The game log is `logs/latest.log` in your game folder; Yak VC's lines contain `(yakvc)`.

### The debug overlay

For testing, or to attach to a bug report, the debug overlay shows what Yak VC is doing in the top-right corner, a bit like F3. Bind **Toggle Debug Overlay** in the key binds to turn it on. It updates four times a second.

| Line | Meaning |
| --- | --- |
| Rendezvous | The connection to the Yak VC server: `connecting`, `authenticating`, `registered` (with how long your sign-in ticket has left), `retrying in` some seconds, or `disconnected`. |
| Account | Whether the server verified your Minecraft account. |
| Endpoint | The start of your Yak VC address, which other players' logs show too; `relay only` when `relay_only` is set; and the version of the voice protocol your Yak VC speaks (`voice v4`). |
| Microphone, Speakers | The device from the settings file, and whether it is open. `not open` means Yak VC can't use it; the game log says why. |
| Level | Your microphone's loudness, and how you talk: push to talk, or voice above the activation threshold. |
| Sending | Whether your voice is going out right now; also `muted` or `deafened`. |
| Bitrate | The quality your voice is sent at now, and the setting. It drops below the setting when the relay is busy. |
| Overruns, underruns | Glitches since the device opened: microphone audio Yak VC was too slow to pick up, and moments the speakers ran dry. A count that keeps rising means crackling. |
| Peers | One line per Yak VC player you are connected to: `direct` or `relayed` (or `connecting`, `failed`, `relay full`), `verified` if their account is, `voice v3` (for example) if their Yak VC speaks another voice version than yours, in which case talking with them may not work until you both run the same version, the round trip time (`rtt`), the share of their voice lost on the way (`loss`), packets that came too late to play (`late`), and how much audio is held back to smooth out uneven arrival (`buffer`). Loss and late appear once they have talked. |

## The settings file

Settings live in `config/yakvc/settings.json` in your game folder. Yak VC writes it with every key the first time it starts. The game reads the file when it starts, so edit it while the game is closed, and restart to apply a change. A key you leave out takes its default, and a key Yak VC doesn't know is ignored.

| Key | Default | Setting |
| --- | --- | --- |
| `voice_range` | `256.0` | How far away players can hear you and you can hear them, in blocks, up to your render distance (16 blocks per chunk) |
| `spatial_audio` | `"full"` | How much of the world's acoustics to hear: `"off"` (volume and left or right only), `"simple"` (walls muffle voices and voices come round them) or `"full"` (rooms also reverberate and echo) |
| `relay_only` | `false` | Never connect to peers directly, so they don't learn your IP address |
| `respect_chat_restrictions` | `true` | Voice off while your account or launcher disables chat |
| `mute_blocked_players` | `true` | Players blocked in Social Interactions are muted both ways |
| `verified_only` | `false` | Only talk with players whose account is verified |
| `invites` | `"everyone"` | Who may invite you to a group: `"everyone"` or `"nobody"` |
| `audio.activation` | `"push_to_talk"` | `"push_to_talk"` or `"voice"` |
| `audio.vad_threshold_db` | `-45.0` | Voice activation threshold, from -100 (everything) to 0 (very loud only) |
| `audio.bitrate` | `24000` | Sound quality, in bits per second |
| `audio.noise_suppression` | `true` | Filters fans, hum and room noise out of your microphone |
| `audio.input_gain` | `1.0` | Microphone volume, from 0 to 10 |
| `audio.input_device` | `null` | Microphone, by device id; `null` follows the system |
| `audio.output_device` | `null` | Speakers, by device id; `null` follows the game's sound device |
| `acoustics` | | How voices carry through the world; see below |
| `server` | `null` | A code from `yakvc-server code`, for [another voice server](#using-another-voice-server) |
| `rendezvous`, `trusted_issuers` | the Yak VC server | Another voice server, spelled out for those who [run their own](../deploy/README.md) without a relay. A `server` code beats them. |

`audio.activation` means the `activation` key inside the `audio` object:

```json
{
  "voice_range": 64.0,
  "audio": {"activation": "voice"}
}
```

The `acoustics` object holds the numbers behind how voices carry: `full_volume_distance` and `distance_rolloff` (how close a voice plays at full volume, and how far away it is half as loud), `occlusion_attenuation` and `muffled_cutoff_hz` (how much quieter and duller a voice behind walls is), `path_bend` and `path_per_block` (what coming round a corner or through a doorway costs), `reverb_level` and `reverb_damping`, `furnishing` (how much sound surfaces soak up beyond their blocks), `max_decay` and `room_reach` (the longest a room rings, and how far around you to look for walls), and `echo_feedback` (how many times the room's echo repeats).

### Modpack presets

A modpack can ship a `config/yakvc/settings.json` with only the keys it wants to preset, and players get the defaults for everything else. A pack for a group with [its own voice server](#using-another-voice-server) ships `{"server": "yakvc1:…"}`.

The same folder holds `node.key`, the private key that identifies your game to other players. Keep it to yourself; if you delete it, Yak VC makes a new one.
