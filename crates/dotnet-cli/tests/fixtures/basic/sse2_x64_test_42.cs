using System;
using System.Runtime.Intrinsics.X86;

public class Program {
    public static int Main() {
        bool supported = Sse2.X64.IsSupported;
        return supported ? 0 : 42;
    }
}
