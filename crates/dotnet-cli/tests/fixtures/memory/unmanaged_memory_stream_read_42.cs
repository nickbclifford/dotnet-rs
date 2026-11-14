using System;
using System.IO;

public unsafe class Program {
    public static int Main() {
        byte* p = (byte*)System.Runtime.InteropServices.NativeMemory.Alloc(64);
        for (int i = 0; i < 64; i++) p[i] = (byte)i;
        var s = new UnmanagedMemoryStream(p, 64);
        byte[] buf = new byte[4];
        int n = s.Read(buf, 0, 4);
        if (n != 4) return 1;
        if (buf[0] != 0 || buf[3] != 3) return 2;
        if (new BinaryReader(s).ReadInt32() != 0x07060504) return 3;
        return 42;
    }
}
