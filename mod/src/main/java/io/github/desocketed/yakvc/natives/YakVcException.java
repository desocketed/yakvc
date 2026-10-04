package io.github.desocketed.yakvc.natives;

/** A native call failed. {@code code} is one of the {@code YAKVC_ERR_*} values in {@code yakvc.h}. */
public final class YakVcException extends RuntimeException {
	private final int code;

	public YakVcException(int code, String message) {
		super(message + " (error " + code + ")");
		this.code = code;
	}

	public int code() {
		return code;
	}

	/** The engine panicked, now or earlier, and can only be destroyed. */
	public boolean poisoned() {
		return code == NativeBridge.ERR_PANIC || code == NativeBridge.ERR_POISONED;
	}
}
