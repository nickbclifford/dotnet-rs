// File: conversions/unsigned_to_float_42.cs
// Purpose: Test conv.r.un instruction
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        uint u = 0xFFFFFFFF;
        double d = (double)u;
        if (d != 4294967295.0) return 1;

        ulong ul = 0xFFFFFFFFFFFFFFFF;
        double d2 = (double)ul;
        // 2^64 - 1 is approximately 1.8446744073709552e+19
        // Precision might be lost, but it should be close.
        if (d2 < 1.84467440737e19) return 2;

        return 42;
    }
}
