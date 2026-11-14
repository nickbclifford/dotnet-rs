using System;
using System.IO;
using System.Runtime.InteropServices;

// A non-generic callee must not inherit the method generics of a generic caller. The Buffer.Memmove
// intrinsic sizes its copy from them: with T = long leaking in, 4 bytes became 4 * sizeof(long).
public unsafe class Program {
    static bool Check<T>(UnmanagedMemoryStream source, byte* destination) {
        var span = new Span<byte>(destination, 4);
        return source.Read(span) == 4;
    }

    public static int Main() {
        byte* source = (byte*)NativeMemory.Alloc(64);
        byte* destination = (byte*)NativeMemory.AllocZeroed(64);
        for (int i = 0; i < 64; i++) source[i] = (byte)(i + 1);

        var stream = new UnmanagedMemoryStream(source, 64);
        if (!Check<long>(stream, destination)) return 1;
        for (int i = 0; i < 4; i++) if (destination[i] != i + 1) return 2;
        for (int i = 4; i < 64; i++) if (destination[i] != 0) return 3;
        return 42;
    }
}
