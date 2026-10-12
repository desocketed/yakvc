package io.github.desocketed.yakvc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.net.URI;
import org.junit.jupiter.api.Test;

class UpdateCheckTest {
	private static int compare(String a, String b) {
		return UpdateCheck.compare(UpdateCheck.parse(a), UpdateCheck.parse(b));
	}

	@Test
	void ordersVersionsAsSemverDoes() {
		assertTrue(compare("0.1.0-alpha.10", "0.1.0-alpha.9") > 0);
		assertTrue(compare("0.1.0", "0.1.0-alpha.9") > 0);
		assertTrue(compare("0.1.0-beta.1", "0.1.0-alpha.9") > 0);
		assertTrue(compare("0.1.0-alpha.1", "0.1.0-alpha") > 0);
		assertTrue(compare("0.1.0-alpha.1", "0.1.0-alpha.x") < 0);
		assertTrue(compare("0.2.0-alpha.1", "0.1.9") > 0);
		assertTrue(compare("0.10.0", "0.9.0") > 0);
		assertEquals(0, compare("v1.2.3+build.5", "1.2.3"));
	}

	@Test
	void rejectsWhatIsNotAVersion() {
		assertNull(UpdateCheck.parse("${version}"));
		assertNull(UpdateCheck.parse("1.2"));
		assertNull(UpdateCheck.parse("1.2.x"));
		assertNull(UpdateCheck.parse("1.2.3-"));
		assertNull(UpdateCheck.parse("1.2.3-alpha..1"));
	}

	private static final String RELEASES = """
			[
			  {"tag_name": "v0.2.0", "draft": true, "prerelease": false, "html_url": "https://example.com/v0.2.0"},
			  {"tag_name": "v0.1.0-alpha.10", "draft": false, "prerelease": true, "html_url": "https://example.com/a10"},
			  {"tag_name": "v0.1.0-alpha.9", "draft": false, "prerelease": true, "html_url": "https://example.com/a9"},
			  {"tag_name": "v0.0.9", "draft": false, "prerelease": false, "html_url": "https://example.com/v0.0.9"}
			]
			""";

	@Test
	void picksTheNewestPrereleaseForAPrerelease() {
		assertEquals(new UpdateCheck.Release("0.1.0-alpha.10", URI.create("https://example.com/a10")),
				UpdateCheck.newerRelease(RELEASES, "0.1.0-alpha.3"));
	}

	@Test
	void skipsPrereleasesForARelease() {
		assertNull(UpdateCheck.newerRelease(RELEASES, "0.0.9"));
		assertEquals("0.0.9", UpdateCheck.newerRelease(RELEASES, "0.0.8").version());
	}

	@Test
	void findsNothingWhenUpToDateOrUnparsable() {
		assertNull(UpdateCheck.newerRelease(RELEASES, "0.1.0-alpha.10"));
		assertNull(UpdateCheck.newerRelease(RELEASES, "0.1.0"));
		assertNull(UpdateCheck.newerRelease(RELEASES, "dev"));
	}
}
