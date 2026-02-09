using System;
using System.Runtime.Intrinsics.X86;

public class Program {
    public static int Main() {
        // Test that Lzcnt.IsSupported doesn't cause infinite recursion
        bool supported = Lzcnt.IsSupported;
        return supported ? 0 : 42;
    }
}
