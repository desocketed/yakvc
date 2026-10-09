package io.github.desocketed.yakvc.natives;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * A voice group from {@code yakvc_list_groups}: ours ({@code mine}, members including us) or a public one a peer
 * announces. {@code id} is 64 hex characters; {@code label} is empty when nobody set one.
 */
public record VoiceGroup(String id, String label, boolean isPublic, boolean mine, List<UUID> members) {
	/** Parses the engine's list: our own group first, if we are in one, then the public groups. */
	public static List<VoiceGroup> parse(String json) {
		List<VoiceGroup> groups = new ArrayList<>();
		for (JsonElement element : JsonParser.parseString(json).getAsJsonArray()) {
			JsonObject fields = element.getAsJsonObject();
			List<UUID> members = new ArrayList<>();
			for (JsonElement member : fields.getAsJsonArray("members")) {
				members.add(UUID.fromString(member.getAsString()));
			}
			groups.add(new VoiceGroup(fields.get("id").getAsString(), fields.get("label").getAsString(),
					fields.get("public").getAsBoolean(), fields.get("mine").getAsBoolean(), List.copyOf(members)));
		}
		return groups;
	}
}
