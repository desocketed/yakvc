// Copies the client gametest's screenshots into docs/guide/ for docs/USER_GUIDE.md, but only the ones that look
// different, so the small rendering differences between machines don't change the repository on every run. CI runs
// it after the gametest on main and commits what changed; locally, run it after scripts/gametest-headless.sh.
//
// Usage (from the repo root): java scripts/UpdateGuideScreenshots.java [screenshots dir] [guide dir]
//
// A pixel counts as changed when a colour channel moves by more than PIXEL_TOLERANCE, and an image as changed when
// more than CHANGED_PIXELS_ALLOWED pixels did, or its size did. Software rendering on different CPUs shifts colours
// by a few steps; a changed label or icon moves many pixels by a lot.

import java.awt.image.BufferedImage;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.List;
import java.util.stream.Stream;
import javax.imageio.ImageIO;

static final int PIXEL_TOLERANCE = 16;
static final int CHANGED_PIXELS_ALLOWED = 20;

void main(String[] args) throws IOException {
	Path from = Path.of(args.length > 0 ? args[0] : "mod/build/run/clientGameTest/screenshots");
	Path to = Path.of(args.length > 1 ? args[1] : "docs/guide");
	List<Path> screenshots;
	try (Stream<Path> files = Files.list(from)) {
		screenshots = files.filter(file -> file.toString().endsWith(".png")).sorted().toList();
	}
	if (screenshots.isEmpty()) throw new IOException("no screenshots in " + from);
	Files.createDirectories(to);
	for (Path screenshot : screenshots) {
		Path current = to.resolve(screenshot.getFileName());
		int changed = Files.exists(current) ? changedPixels(screenshot.toFile(), current.toFile()) : Integer.MAX_VALUE;
		if (changed > CHANGED_PIXELS_ALLOWED) {
			Files.copy(screenshot, current, StandardCopyOption.REPLACE_EXISTING);
			IO.println("updated   " + current + (changed == Integer.MAX_VALUE ? "" : " (" + changed + " pixels changed)"));
		} else {
			IO.println("unchanged " + current + " (" + changed + " pixels changed)");
		}
	}
}

/** How many pixels differ visibly, or Integer.MAX_VALUE if the sizes differ. */
int changedPixels(File a, File b) throws IOException {
	BufferedImage imageA = ImageIO.read(a);
	BufferedImage imageB = ImageIO.read(b);
	if (imageA.getWidth() != imageB.getWidth() || imageA.getHeight() != imageB.getHeight()) return Integer.MAX_VALUE;
	int changed = 0;
	for (int y = 0; y < imageA.getHeight(); y++) {
		for (int x = 0; x < imageA.getWidth(); x++) {
			int rgbA = imageA.getRGB(x, y);
			int rgbB = imageB.getRGB(x, y);
			for (int shift = 0; shift <= 16; shift += 8) {
				if (Math.abs((rgbA >> shift & 0xFF) - (rgbB >> shift & 0xFF)) > PIXEL_TOLERANCE) {
					changed++;
					break;
				}
			}
		}
	}
	return changed;
}
