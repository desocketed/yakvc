package io.github.desocketed.yakvc;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import org.jspecify.annotations.Nullable;

/**
 * Asks GitHub once per game whether a newer Yak VC is out. The request is a plain GET of the public releases list;
 * nothing about the player is sent, and any failure only means no notice.
 */
public final class UpdateCheck {
	private static final URI RELEASES = URI.create("https://api.github.com/repos/desocketed/yakvc/releases?per_page=10");
	private static final Duration TIMEOUT = Duration.ofSeconds(10);

	/** A release newer than the running one. */
	public record Release(String version, URI page) {}

	private UpdateCheck() {}

	/**
	 * Fetches the releases in the background and completes with a newer release than {@code running}, or null if
	 * there is none or anything went wrong (logged once). Never blocks the caller.
	 */
	public static CompletableFuture<@Nullable Release> start(String running) {
		HttpRequest request = HttpRequest.newBuilder(RELEASES)
				.timeout(TIMEOUT)
				.header("User-Agent", "yakvc/" + running)
				.header("Accept", "application/vnd.github+json")
				.build();
		return HttpClient.newBuilder().connectTimeout(TIMEOUT).build()
				.sendAsync(request, HttpResponse.BodyHandlers.ofString())
				.thenApply(response -> {
					if (response.statusCode() != 200) {
						throw new IllegalStateException("GitHub answered " + response.statusCode());
					}
					return newerRelease(response.body(), running);
				})
				.exceptionally(e -> {
					YakVcClient.LOGGER.info("Could not check for a Yak VC update: {}", e.toString());
					return null;
				});
	}

	/**
	 * The newest release in GitHub's releases JSON that is newer than {@code running}, or null. Drafts never count;
	 * prereleases only when {@code running} is one itself, so someone on a stable release isn't pushed to an alpha.
	 */
	static @Nullable Release newerRelease(String json, String running) {
		List<Object> current = parse(running);
		if (current == null) return null;
		boolean wantPrereleases = current.size() > 3;
		Release best = null;
		List<Object> bestVersion = current;
		for (JsonElement element : JsonParser.parseString(json).getAsJsonArray()) {
			JsonObject release = element.getAsJsonObject();
			if (release.get("draft").getAsBoolean()) continue;
			if (release.get("prerelease").getAsBoolean() && !wantPrereleases) continue;
			String tag = release.get("tag_name").getAsString();
			List<Object> version = parse(tag);
			if (version == null || compare(version, bestVersion) <= 0) continue;
			best = new Release(tag.startsWith("v") ? tag.substring(1) : tag,
					URI.create(release.get("html_url").getAsString()));
			bestVersion = version;
		}
		return best;
	}

	/**
	 * A semantic version as its parts: major, minor and patch as Integers, then the prerelease identifiers (Integers
	 * where numeric, so alpha.10 sorts after alpha.9). Null if it isn't one, such as a dev build's version.
	 */
	static @Nullable List<Object> parse(String text) {
		String version = text.startsWith("v") ? text.substring(1) : text;
		int plus = version.indexOf('+');
		if (plus >= 0) version = version.substring(0, plus); // Build metadata doesn't order versions.
		int dash = version.indexOf('-');
		String core = dash >= 0 ? version.substring(0, dash) : version;
		String[] numbers = core.split("\\.", -1);
		if (numbers.length != 3) return null;
		List<Object> parts = new ArrayList<>();
		for (String number : numbers) {
			if (!number.matches("\\d+")) return null;
			parts.add(Integer.parseInt(number));
		}
		if (dash >= 0) {
			for (String identifier : version.substring(dash + 1).split("\\.", -1)) {
				if (identifier.isEmpty()) return null;
				parts.add(identifier.matches("\\d+") ? Integer.parseInt(identifier) : identifier);
			}
		}
		return parts;
	}

	/** Orders two parsed versions as semantic versioning does. */
	static int compare(List<Object> a, List<Object> b) {
		for (int i = 0; i < 3; i++) {
			int order = Integer.compare((Integer) a.get(i), (Integer) b.get(i));
			if (order != 0) return order;
		}
		// A release sorts after its own prereleases.
		boolean aRelease = a.size() == 3;
		boolean bRelease = b.size() == 3;
		if (aRelease || bRelease) return Boolean.compare(aRelease, bRelease);
		for (int i = 3; i < Math.min(a.size(), b.size()); i++) {
			int order = compareIdentifier(a.get(i), b.get(i));
			if (order != 0) return order;
		}
		return Integer.compare(a.size(), b.size());
	}

	/** Numbers sort numerically and before words; words sort as text. */
	private static int compareIdentifier(Object a, Object b) {
		if (a instanceof Integer x && b instanceof Integer y) return Integer.compare(x, y);
		if (a instanceof Integer) return -1;
		if (b instanceof Integer) return 1;
		return ((String) a).compareTo((String) b);
	}
}
