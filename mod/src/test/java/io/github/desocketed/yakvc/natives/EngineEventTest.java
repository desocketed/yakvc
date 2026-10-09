package io.github.desocketed.yakvc.natives;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.UUID;
import org.junit.jupiter.api.Test;

class EngineEventTest {
	private static final UUID ID = UUID.fromString("00112233-4455-6677-8899-aabbccddeeff");
	private static final byte[] UUID_BYTES = NativeBridge.uuidBytes(List.of(ID));

	@Test
	void decodesEveryRecordType() {
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		record(out, 1, le(4).putInt(7).array(), "-7f".getBytes(StandardCharsets.UTF_8));
		record(out, 2, new byte[] {2}, le(8).putLong(1_800_000_000L).array(), new byte[] {1});
		record(out, 3, UUID_BYTES, new byte[] {1, 0});
		record(out, 4, UUID_BYTES, new byte[] {1});
		record(out, 5, le(4).putFloat(-12.5f).array());
		record(out, 99, new byte[] {1, 2, 3});
		record(out, 6, "no mic".getBytes(StandardCharsets.UTF_8));
		byte[] buf = out.toByteArray();

		assertEquals(List.of(
				new EngineEvent.JoinRequest(7, "-7f"),
				new EngineEvent.Rendezvous(EngineEvent.RendezvousState.REGISTERED, 1_800_000_000L, true),
				new EngineEvent.Peer(ID, EngineEvent.PeerState.DIRECT, false),
				new EngineEvent.Talking(ID, true),
				new EngineEvent.MicLevel(-12.5f),
				new EngineEvent.Error("no mic")), EngineEvent.decode(buf, buf.length));
	}

	@Test
	void decodesGroupEvents() {
		UUID other = UUID.fromString("ffeeddcc-bbaa-9988-7766-554433221100");
		byte[] otherBytes = NativeBridge.uuidBytes(List.of(other));
		byte[] nil = new byte[16];
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		record(out, 7, UUID_BYTES);
		record(out, 8, new byte[] {0}, UUID_BYTES, nil);
		record(out, 8, new byte[] {4}, nil, otherBytes);
		record(out, 8, new byte[] {9}, UUID_BYTES, nil);
		byte[] buf = out.toByteArray();

		assertEquals(List.of(
				new EngineEvent.Invite(ID),
				new EngineEvent.GroupNotice(EngineEvent.NoticeKind.JOINED, ID, new UUID(0, 0)),
				new EngineEvent.GroupNotice(EngineEvent.NoticeKind.LABEL_CHANGED, new UUID(0, 0), other)),
				EngineEvent.decode(buf, buf.length));
	}

	@Test
	void skipsUnknownStateCodes() {
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		record(out, 2, new byte[] {(byte) 200}, le(8).putLong(0).array(), new byte[] {0});
		record(out, 3, UUID_BYTES, new byte[] {-1, 0});
		record(out, 4, UUID_BYTES, new byte[] {1});
		byte[] buf = out.toByteArray();

		assertEquals(List.of(new EngineEvent.Talking(ID, true)), EngineEvent.decode(buf, buf.length));
	}

	@Test
	void skipsShortPayloads() {
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		record(out, 4, new byte[] {1, 2, 3});
		record(out, 5, le(4).putFloat(-3f).array());
		byte[] buf = out.toByteArray();

		assertEquals(List.of(new EngineEvent.MicLevel(-3f)), EngineEvent.decode(buf, buf.length));
	}

	@Test
	void keepsEventsBeforeATruncatedRecord() {
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		record(out, 5, le(4).putFloat(-3f).array());
		record(out, 4, UUID_BYTES, new byte[] {1});
		byte[] buf = out.toByteArray();

		assertEquals(List.of(new EngineEvent.MicLevel(-3f)), EngineEvent.decode(buf, buf.length - 1));
	}

	private static ByteBuffer le(int size) {
		return ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN);
	}

	private static void record(ByteArrayOutputStream out, int type, byte[]... parts) {
		int len = 0;
		for (byte[] part : parts) len += part.length;
		out.writeBytes(le(4).putShort((short) type).putShort((short) len).array());
		for (byte[] part : parts) out.writeBytes(part);
	}
}
