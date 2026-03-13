using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Explicit)]
struct ExplicitUnion {
    [FieldOffset(0)] public byte b1;
    [FieldOffset(0)] public int i1;
}

[StructLayout(LayoutKind.Explicit, Size = 16)]
struct ExplicitSize16 {
    [FieldOffset(0)] public byte b1;
}

[StructLayout(LayoutKind.Explicit)]
struct ExplicitOverlapRef {
    [FieldOffset(0)] public object o1;
    [FieldOffset(0)] public object o2;
}

[StructLayout(LayoutKind.Explicit)]
class ExplicitBase {
    [FieldOffset(0)] public int x;
}

[StructLayout(LayoutKind.Explicit)]
class ExplicitDerived : ExplicitBase {
    [FieldOffset(0)] public int y;
}

public class Program {
    public static int Main() {
        // ExplicitUnion: b1 and i1 both at 0, size 4
        if ((int)Marshal.OffsetOf<ExplicitUnion>("b1") != 0) return 1;
        if ((int)Marshal.OffsetOf<ExplicitUnion>("i1") != 0) return 2;
        if (Marshal.SizeOf<ExplicitUnion>() != 4) return 3;

        // ExplicitSize16: size 16
        if (Marshal.SizeOf<ExplicitSize16>() != 16) return 4;

        // ExplicitOverlapRef: both at 0
        if ((int)Marshal.OffsetOf<ExplicitOverlapRef>("o1") != 0) return 5;
        if ((int)Marshal.OffsetOf<ExplicitOverlapRef>("o2") != 0) return 6;

        // ExplicitBase/Derived:
        // Note: OffsetOf on classes might include the overhead (SyncBlock/MethodTable) 
        // depending on the runtime. In dotnet-rs, let's see what it does.
        // Actually, let's just check relative offsets if possible.
        
        int baseOffsetX = (int)Marshal.OffsetOf<ExplicitBase>("x");
        int derivedOffsetX = (int)Marshal.OffsetOf<ExplicitDerived>("x");
        int derivedOffsetY = (int)Marshal.OffsetOf<ExplicitDerived>("y");

        if (baseOffsetX != derivedOffsetX) return 7;
        
        // y should be after x. If int is 4 bytes, y should be at x + 4.
        if (derivedOffsetY <= derivedOffsetX) return 8;
        if (derivedOffsetY != derivedOffsetX + 4) return 9;

        return 0;
    }
}
