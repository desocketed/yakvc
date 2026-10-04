package io.github.desocketed.yakvc.natives;

import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;

/**
 * Downcall handles for the C ABI in {@code crates/yakvc-ffi/include/yakvc.h}, plus thin typed wrappers.
 */
public final class NativeBridge {
	/** Must equal {@code YAKVC_ABI_VERSION} in {@code yakvc.h}. */
	public static final int ABI_VERSION = 1;

	private final MethodHandle abiVersion;

	public NativeBridge(SymbolLookup library) {
		Linker linker = Linker.nativeLinker();
		this.abiVersion = linker.downcallHandle(
				find(library, "yakvc_abi_version"),
				FunctionDescriptor.of(ValueLayout.JAVA_INT));
	}

	/** Returns the library's {@code YAKVC_ABI_VERSION}. */
	public int abiVersion() {
		try {
			return (int) abiVersion.invokeExact();
		} catch (Throwable t) {
			throw new AssertionError("yakvc_abi_version cannot throw", t);
		}
	}

	private static java.lang.foreign.MemorySegment find(SymbolLookup library, String name) {
		return library.find(name).orElseThrow(() -> new UnsatisfiedLinkError("missing native symbol " + name));
	}
}
