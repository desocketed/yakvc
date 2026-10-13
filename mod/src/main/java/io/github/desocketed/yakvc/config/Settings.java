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
import java.util.List;
import java.util.Map;
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
 * @param devMode                 accept dev tickets and test audio, for development against a local server
 * @param friendsOnly             when set, only these players (UUIDs) are heard and sent to
 * @param rendezvous              another voice server than the built-in one
 * @param trustedIssuers          issuers whose tickets are accepted; a {@code rendezvous} needs its issuer listed
 */
public record Settings(double voiceRange, SpatialAudio spatialAudio, boolean relayOnly, boolean respectChatRestrictions,
		boolean muteBlockedPlayers, boolean verifiedOnly, String invites, boolean setupDone, boolean checkForUpdates,
		Audio audio, Acoustics acoustics, boolean devMode, Dev dev, @Nullable List<String> friendsOnly,
		@Nullable Rendezvous rendezvous, List<String> trustedIssuers) {
	public static final String FILE_NAME = "settings.json";
	public static final Settings DEFAULT = new Settings(256, SpatialAudio.SIMPLE, false, true, true, false, "everyone",
			false, true, Audio.DEFAULT, Acoustics.DEFAULT, false, Dev.DEFAULT, null, null, List.of());
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
	 */
	public record Dev(@Nullable Double toneHz, @Nullable String wav, boolean nullOutput) {
		public static final Dev DEFAULT = new Dev(null, null, false);
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

	public Settings withSetupDone(boolean done) {
		return with("", "setup_done", done);
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
	 * constructor call naming all sixteen components. {@code section} is "" for a top-level key.
	 */
	private Settings with(String section, String key, @Nullable Object value) {
		JsonObject json = GSON.toJsonTree(this).getAsJsonObject();
		JsonObject target = section.isEmpty() ? json : json.getAsJsonObject(section);
		if (!target.has(key)) throw new IllegalArgumentException("no setting " + key);
		target.add(key, GSON.toJsonTree(value));
		return GSON.fromJson(json, Settings.class);
	}
}
