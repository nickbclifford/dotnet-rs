using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)]
struct SequentialDefault {
    public byte b1;
    public int i1;
}

[StructLayout(LayoutKind.Sequential, Pack = 1)]
struct SequentialPack1 {
    public byte b1;
    public int i1;
}

[StructLayout(LayoutKind.Sequential, Pack = 2)]
struct SequentialPack2 {
    public byte b1;
    public int i1;
}

[StructLayout(LayoutKind.Sequential, Size = 16)]
struct SequentialSize16 {
    public byte b1;
}

[StructLayout(LayoutKind.Sequential)]
struct Nested {
    public SequentialPack2 s1;
    public byte b2;
}

public class Program {
    public static int Main() {
        // SequentialDefault: expect b1 at 0, i1 at 4, size 8
        if ((int)Marshal.OffsetOf<SequentialDefault>("b1") != 0) return 1;
        if ((int)Marshal.OffsetOf<SequentialDefault>("i1") != 4) return 2;
        if (Marshal.SizeOf<SequentialDefault>() != 8) return 3;

        // SequentialPack1: expect b1 at 0, i1 at 1, size 5
        if ((int)Marshal.OffsetOf<SequentialPack1>("b1") != 0) return 4;
        if ((int)Marshal.OffsetOf<SequentialPack1>("i1") != 1) return 5;
        if (Marshal.SizeOf<SequentialPack1>() != 5) return 6;

        // SequentialPack2: expect b1 at 0, i1 at 2, size 6
        if ((int)Marshal.OffsetOf<SequentialPack2>("b1") != 0) return 7;
        if ((int)Marshal.OffsetOf<SequentialPack2>("i1") != 2) return 8;
        if (Marshal.SizeOf<SequentialPack2>() != 6) return 9;

        // SequentialSize16: expect size 16
        if (Marshal.SizeOf<SequentialSize16>() != 16) return 10;

        // Nested: s1 (size 6, align 2) at 0, b2 at 6, total size 8
        if ((int)Marshal.OffsetOf<Nested>("s1") != 0) return 11;
        if ((int)Marshal.OffsetOf<Nested>("b2") != 6) return 12;
        if (Marshal.SizeOf<Nested>() != 8) return 13;

        return 0;
    }
}
