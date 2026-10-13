package io.github.desocketed.yakvc.config;

import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import static java.util.stream.Collectors.joining;

import java.util.List;
import java.util.Map;
import java.util.UUID;
import org.jspecify.annotations.Nullable;

/**
 * Every setting, kept in {@code config/yakvc/settings.json}. The file is written whole, so one with only some keys,
 * such as a modpack's preset, takes the defaults for the rest, and keys the mod doesn't know are dropped on the next
 * save. The keys are the engine's own names (snake case), so one vocabulary covers the guide, the file and the engine,
 * which gets the settings it uses as TOML text from {@link #engineToml}.
 *
 * @param voiceRange              blocks; between two players the shorter range applies
 * @param spatialAudio            how much of the world's acoustics to hear
 * @param relayOnly               never connect to peers directly, so they don't learn our IP address
 * @param respectChatRestrictions voice off while the account or launcher disables chat
 * @param muteBlockedPlayers      players blocked in Social Interactions are muted both ways
 * @param verifiedOnly            only talk with players whose account is verified with Mojang
 * @param invites                 who may invite us to a group: {@code "everyone"}, {@code "friends"} or {@code "nobody"}
 * @param setupDone               the voice menu's first-run card was dismissed
 * @param checkForUpdates         ask GitHub for a newer Yak VC at startup
 * @param friendsOnly             only the players marked as friends in the voice menu are heard and sent to
 * @param devMode                 accept dev tickets and test audio, for development against a local server
 * @param rendezvous              another voice server than the built-in one
 * @param trustedIssuers          issuers whose tickets are accepted; a {@code rendezvous} needs its issuer listed
 * @param server                  a server code from {@code yakvc-server code}, the Voice Server setting; beats
 *                                {@code rendezvous}, and null means the preset or the built-in server
 */
public record Settings(double voiceRange, SpatialAudio spatialAudio, boolean relayOnly, boolean respectChatRestrictions,
		boolean muteBlockedPlayers, boolean verifiedOnly, String invites, boolean friendsOnly, boolean setupDone,
		boolean checkForUpdates, Audio audio, Acoustics acoustics, boolean devMode, Dev dev,
		@Nullable Rendezvous rendezvous, List<String> trustedIssuers, @Nullable String server) {
	public static final String FILE_NAME = "settings.json";
	public static final Settings DEFAULT = new Settings(256, SpatialAudio.SIMPLE, false, true, true, false, "everyone",
			false, false, true, Audio.DEFAULT, Acoustics.DEFAULT, false, Dev.DEFAULT, null, List.of(), null);
	/** Nulls are written too, so the file shows every key, and the JSON keys are the engine's snake case. */
	private static final Gson GSON = new GsonBuilder().setPrettyPrinting().serializeNulls()
			.setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES).create();

	/**
	 * @param inputDevice    the microphone's id, or null for the system default
	 * @param outputDevice   the speakers' id, or null to follow the game's sound device
	 * @param activation     {@code "push_to_talk"} or {@code "voice"}
	 * @param bitrate        Opus bits per second
	 * @param inputGain      linear gain on the microphone, 1 being unchanged
	 * @param vadThresholdDb dBFS above which voice activation transmits
	 */
	public record Audio(@Nullable String inputDevice, @Nullable String outputDevice, String activation, int bitrate,
			boolean noiseSuppression, double inputGain, double vadThresholdDb) {
		public static final Audio DEFAULT = new Audio(null, null, "push_to_talk", 24_000, true, 1, -45);

		public boolean voiceActivation() {
			return activation.equals("voice");
		}
	}

	/**
	 * How voices sound through the world, for tuning by ear. The first eight are the engine's ({@code AcousticsConfig}),
	 * the rest the mod's ({@link SoundTuning}: how it reads rooms and walls from the world).
	 */
	public record Acoustics(double fullVolumeDistance, double distanceRolloff, double occlusionAttenuation,
			double muffledCutoffHz, double reverbLevel, double reverbDamping, double echoFeedback, double echoLevel,
			double furnishing, double maxDecay, double roomReach, double echoReach, double pathBend, double pathPerBlock) {
		public static final Acoustics DEFAULT = new Acoustics(4, 3, 0, 600, 0.35, 0.25, 0.12, 1, 0.12, 6, 64, 200, 0.4,
				1 / 24.0);
	}

	/**
	 * Test audio instead of devices, for machines without a sound card; needs {@code dev_mode}.
	 *
	 * @param toneHz     a sine tone instead of the microphone
	 * @param wav        a WAV file, looping, instead of the microphone
	 * @param nullOutput discard the output instead of opening speakers
	 * @param flySpeed   creative flight speed while set, for getting around a big dev world (vanilla is 0.05)
	 */
	public record Dev(@Nullable Double toneHz, @Nullable String wav, boolean nullOutput, @Nullable Double flySpeed) {
		public static final Dev DEFAULT = new Dev(null, null, false, null);
	}

	/** A voice server: its endpoint id, direct addresses ({@code host:port}) and, if it has one, its relay URL. */
	public record Rendezvous(String endpointId, List<String> addrs, @Nullable String relay) {}

	/** Reads the settings, writing the defaults first if there is no file. */
	public static Settings load(Path configDir) throws IOException {
		Path file = configDir.resolve(FILE_NAME);
		if (!Files.exists(file)) {
			DEFAULT.save(configDir);
			return DEFAULT;
		}
		return parse(Files.readString(file));
	}

	/** The settings in {@code json}, with the defaults for every key it leaves out. */
	static Settings parse(String json) throws IOException {
		try {
			JsonObject settings = GSON.toJsonTree(DEFAULT).getAsJsonObject();
			merge(settings, JsonParser.parseString(json).getAsJsonObject());
			Settings parsed = GSON.fromJson(settings, Settings.class);
			// Gson turns a value that is no level into null rather than complaining.
			if (parsed.spatialAudio() == null) throw new IOException("spatial_audio must be \"off\", \"simple\" or \"full\"");
			return parsed;
		} catch (RuntimeException e) {
			throw new IOException(FILE_NAME + ": " + e.getMessage(), e);
		}
	}

	/** Puts {@code from}'s keys into {@code into}, section by section, so a partial {@code audio} keeps the rest. */
	private static void merge(JsonObject into, JsonObject from) {
		for (Map.Entry<String, JsonElement> entry : from.entrySet()) {
			if (into.get(entry.getKey()) instanceof JsonObject section && entry.getValue() instanceof JsonObject theirs) {
				merge(section, theirs);
			} else {
				into.add(entry.getKey(), entry.getValue());
			}
		}
	}

	public void save(Path configDir) throws IOException {
		AtomicFile.write(configDir.resolve(FILE_NAME), GSON.toJson(this) + "\n");
	}

	/**
	 * The voice range in effect: never past the render distance, because players beyond it aren't tracked. Only the
	 * engine gets the capped range, so lowering the render distance for a while keeps the saved one.
	 */
	public static double effectiveVoiceRange(double voiceRange, int renderChunks) {
		return Math.min(voiceRange, renderChunks * 16);
	}

	/**
	 * The settings as the engine takes them ({@code yakvc_create}, {@code yakvc_update_config}): TOML text of the keys
	 * it reads, with the voice range capped by the render distance ({@code renderChunks}; 0 before the first tick, so
	 * uncapped). The engine would ignore the mod's own keys, but leaving them out keeps the text to what it checks.
	 * {@code friends} are the players marked in the voice menu ({@link PlayerVolumes#friends}), which live in
	 * players.json rather than here.
	 */
	public String engineToml(int renderChunks, List<UUID> friends) {
		StringBuilder toml = new StringBuilder();
		key(toml, "voice_range", renderChunks == 0 ? voiceRange : effectiveVoiceRange(voiceRange, renderChunks));
		key(toml, "relay_only", relayOnly);
		key(toml, "verified_only", verifiedOnly);
		key(toml, "invites", invites);
		key(toml, "friends_only", friendsOnly);
		key(toml, "friends", friends.stream().map(UUID::toString).toList());
		key(toml, "dev_mode", devMode);
		key(toml, "trusted_issuers", trustedIssuers);
		if (server != null) key(toml, "server", server);

		toml.append("\n[audio]\n");
		if (audio.inputDevice() != null) key(toml, "input_device", audio.inputDevice());
		if (audio.outputDevice() != null) key(toml, "output_device", audio.outputDevice());
		key(toml, "activation", audio.activation());
		key(toml, "bitrate", audio.bitrate());
		key(toml, "noise_suppression", audio.noiseSuppression());
		key(toml, "input_gain", audio.inputGain());
		key(toml, "vad_threshold_db", audio.vadThresholdDb());

		toml.append("\n[acoustics]\n");
		key(toml, "full_volume_distance", acoustics.fullVolumeDistance());
		key(toml, "distance_rolloff", acoustics.distanceRolloff());
		key(toml, "occlusion_attenuation", acoustics.occlusionAttenuation());
		key(toml, "muffled_cutoff_hz", acoustics.muffledCutoffHz());
		key(toml, "reverb_level", acoustics.reverbLevel());
		key(toml, "reverb_damping", acoustics.reverbDamping());
		key(toml, "echo_feedback", acoustics.echoFeedback());
		key(toml, "echo_level", acoustics.echoLevel());

		if (rendezvous != null) {
			toml.append("\n[rendezvous]\n");
			key(toml, "endpoint_id", rendezvous.endpointId());
			key(toml, "addrs", rendezvous.addrs());
			if (rendezvous.relay() != null) key(toml, "relay", rendezvous.relay());
		}
		// fly_speed is the mod's own; the engine never sees it.
		if (dev.toneHz() != null || dev.wav() != null || dev.nullOutput()) {
			toml.append("\n[dev]\n");
			if (dev.toneHz() != null) key(toml, "tone_hz", dev.toneHz());
			if (dev.wav() != null) key(toml, "wav", dev.wav());
			key(toml, "null_output", dev.nullOutput());
		}
		return toml.toString();
	}

	private static void key(StringBuilder toml, String name, Object value) {
		toml.append(name).append(" = ").append(literal(value)).append('\n');
	}

	/** A TOML literal: integers for whole-number types, floats otherwise, and arrays of strings. */
	private static String literal(Object value) {
		return switch (value) {
			case Boolean b -> b.toString();
			case Integer i -> i.toString();
			// Double.toString keeps a decimal point or an exponent, which is what a TOML float needs.
			case Double d -> d.toString();
			case String s -> quote(s);
			case List<?> list -> list.stream().map(item -> quote((String) item)).collect(joining(", ", "[", "]"));
			default -> throw new IllegalArgumentException(value.getClass().getName());
		};
	}

	/** A TOML basic string, with control characters escaped, so any device name or path reads back as itself. */
	static String quote(String text) {
		StringBuilder out = new StringBuilder("\"");
		for (char c : text.toCharArray()) {
			switch (c) {
				case '"' -> out.append("\\\"");
				case '\\' -> out.append("\\\\");
				case '\n' -> out.append("\\n");
				case '\r' -> out.append("\\r");
				case '\t' -> out.append("\\t");
				default -> {
					if (c < 0x20 || c == 0x7f) out.append(String.format("\\u%04X", (int) c));
					else out.append(c);
				}
			}
		}
		return out.append('"').toString();
	}

	public Settings withVoiceRange(double blocks) {
		return with("", "voice_range", blocks);
	}

	public Settings withSpatialAudio(SpatialAudio level) {
		return with("", "spatial_audio", level.key());
	}

	public Settings withRelayOnly(boolean on) {
		return with("", "relay_only", on);
	}

	public Settings withRespectChatRestrictions(boolean on) {
		return with("", "respect_chat_restrictions", on);
	}

	public Settings withMuteBlockedPlayers(boolean on) {
		return with("", "mute_blocked_players", on);
	}

	public Settings withVerifiedOnly(boolean on) {
		return with("", "verified_only", on);
	}

	public Settings withInvites(String from) {
		return with("", "invites", from);
	}

	public Settings withFriendsOnly(boolean on) {
		return with("", "friends_only", on);
	}

	public Settings withSetupDone(boolean done) {
		return with("", "setup_done", done);
	}

	/** The voice server's code, or null for the preset or built-in one. */
	public Settings withServer(@Nullable String code) {
		return with("", "server", code);
	}

	public Settings withCheckForUpdates(boolean on) {
		return with("", "check_for_updates", on);
	}

	public Settings withActivation(String activation) {
		return with("audio", "activation", activation);
	}

	public Settings withBitrate(int bitsPerSecond) {
		return with("audio", "bitrate", bitsPerSecond);
	}

	public Settings withNoiseSuppression(boolean on) {
		return with("audio", "noise_suppression", on);
	}

	public Settings withInputGain(double gain) {
		return with("audio", "input_gain", gain);
	}

	public Settings withVadThresholdDb(double db) {
		return with("audio", "vad_threshold_db", db);
	}

	public Settings withInputDevice(@Nullable String id) {
		return with("audio", "input_device", id);
	}

	public Settings withOutputDevice(@Nullable String id) {
		return with("audio", "output_device", id);
	}

	/** One {@link Acoustics} value by its key, as the Sound Tuning screen's sliders are named. */
	public Settings withAcoustic(String key, double value) {
		return with("acoustics", key, value);
	}

	/**
	 * A copy with one value changed, by way of the JSON form: each setting changed by hand would otherwise need a
	 * constructor call naming all seventeen components. {@code section} is "" for a top-level key.
	 */
	private Settings with(String section, String key, @Nullable Object value) {
		JsonObject json = GSON.toJsonTree(this).getAsJsonObject();
		JsonObject target = section.isEmpty() ? json : json.getAsJsonObject(section);
		if (!target.has(key)) throw new IllegalArgumentException("no setting " + key);
		target.add(key, GSON.toJsonTree(value));
		return GSON.fromJson(json, Settings.class);
	}
}
