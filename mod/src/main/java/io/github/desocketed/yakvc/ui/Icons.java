package io.github.desocketed.yakvc.ui;

import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FontDescription;
import net.minecraft.resources.Identifier;

/**
 * 8-pixel icons from the {@code yakvc:icons} bitmap font ({@code assets/yakvc/font/icons.json}), so they draw as text:
 * over heads like a name tag. All are white except the speaker, which is green, and the verified badge, which is
 * aqua.
 */
public final class Icons {
	private static final FontDescription FONT = new FontDescription.Resource(Identifier.fromNamespaceAndPath("yakvc", "icons"));

	public static final Component SPEAKER = icon('');
	/** Shown next to players whose account is verified with Mojang. */
	public static final Component VERIFIED = icon('');
	/** Shown next to players in the same voice group as us. */
	public static final Component GROUP = icon('');

	private Icons() {}

	private static Component icon(char c) {
		return Component.literal(String.valueOf(c)).withStyle(style -> style.withFont(FONT));
	}
}
