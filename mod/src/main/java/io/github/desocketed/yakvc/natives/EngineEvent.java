package io.github.desocketed.yakvc.natives;

import java.nio.BufferUnderflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/** An event from the engine, decoded from the TLV records that {@code yakvc_poll_events} writes. */
public sealed interface EngineEvent {
	/** Call Mojang {@code joinServer} with {@code serverId}, then {@code completeJoin(id, ok)}. */
	record JoinRequest(int id, String serverId) implements EngineEvent {}

	/**
	 * {@code value} is the ticket expiry (Unix seconds) when registered, the delay (ms) when retrying, else 0.
	 * {@code verified} is whether a registered session's ticket proves the Mojang account.
	 */
	record Rendezvous(RendezvousState state, long value, boolean verified) implements EngineEvent {}

	/** {@code verified}: the peer proved its Mojang account. */
	record Peer(UUID uuid, PeerState state, boolean verified) implements EngineEvent {}

	/** A peer, or the local player, started or stopped talking. */
	record Talking(UUID uuid, boolean talking) implements EngineEvent {}

	/** Local microphone level in dBFS, about 10 times a second. */
	record MicLevel(float db) implements EngineEvent {}

	/** A non-fatal problem worth showing the user. */
	record Error(String message) implements EngineEvent {}

	/** In {@code YAKVC_RDV_*} order. */
	enum RendezvousState { CONNECTING, AUTHENTICATING, REGISTERED, RETRYING, DISCONNECTED }

	/** In {@code YAKVC_PEER_*} order. */
	enum PeerState { CONNECTING, DIRECT, RELAYED, RELAY_FULL, FAILED, GONE }

	/**
	 * Decodes the first {@code length} bytes of {@code buf}. Unknown record types and state codes are skipped, as are
	 * short payloads, so one bad record doesn't cost the rest of the batch (which may hold a {@link JoinRequest}).
	 */
	static List<EngineEvent> decode(byte[] buf, int length) {
		ByteBuffer in = ByteBuffer.wrap(buf, 0, length).order(ByteOrder.LITTLE_ENDIAN);
		List<EngineEvent> events = new ArrayList<>();
		while (in.remaining() >= 4) {
			int type = Short.toUnsignedInt(in.getShort());
			int len = Short.toUnsignedInt(in.getShort());
			// Records are never split, so this one is corrupt and nothing after it can be trusted.
			if (len > in.remaining()) break;
			ByteBuffer payload = in.slice(in.position(), len).order(ByteOrder.LITTLE_ENDIAN);
			in.position(in.position() + len);
			try {
				EngineEvent event = switch (type) {
					case 1 -> new JoinRequest(payload.getInt(), utf8(payload));
					case 2 -> {
						RendezvousState state = lookup(RendezvousState.values(), payload.get());
						long value = payload.getLong();
						boolean verified = payload.get() != 0;
						yield state == null ? null : new Rendezvous(state, value, verified);
					}
					case 3 -> {
						UUID uuid = uuid(payload);
						PeerState state = lookup(PeerState.values(), payload.get());
						boolean verified = payload.get() != 0;
						yield state == null ? null : new Peer(uuid, state, verified);
					}
					case 4 -> new Talking(uuid(payload), payload.get() != 0);
					case 5 -> new MicLevel(payload.getFloat());
					case 6 -> new Error(utf8(payload));
					default -> null;
				};
				if (event != null) events.add(event);
			} catch (BufferUnderflowException e) {
				// The payload is shorter than its type needs: skip just this record.
			}
		}
		return events;
	}

	/** The constant for an unsigned state code, or null for a code this version doesn't know. */
	private static <E> E lookup(E[] values, byte code) {
		int index = Byte.toUnsignedInt(code);
		return index < values.length ? values[index] : null;
	}

	private static UUID uuid(ByteBuffer in) {
		// UUIDs are big-endian, unlike the rest of the record.
		return new UUID(Long.reverseBytes(in.getLong()), Long.reverseBytes(in.getLong()));
	}

	private static String utf8(ByteBuffer in) {
		byte[] bytes = new byte[in.remaining()];
		in.get(bytes);
		return new String(bytes, StandardCharsets.UTF_8);
	}
}
