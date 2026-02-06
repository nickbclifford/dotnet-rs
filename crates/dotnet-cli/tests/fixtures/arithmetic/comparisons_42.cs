// File: arithmetic/comparisons_42.cs
// Purpose: Test comparison instructions
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        int a = 10;
        int b = 20;
        
        if (a >= b) return 1;
        if (a > b) return 2;
        if (!(a < b)) return 3;
        if (!(a <= b)) return 4;
        if (a == b) return 5;
        if (!(a != b)) return 6;
        
        // Unsigned comparisons
        uint ua = 0xFFFFFFFF;
        uint ub = 1;
        if (ua < ub) return 7;
        if (!(ua > ub)) return 8;

        // Floating point comparisons
        double da = 1.5;
        double db = 2.5;
        if (da > db) return 9;
        if (!(da < db)) return 10;
        
        // NaN comparisons (fe.clt.un etc)
        double nan = double.NaN;
        if (nan == nan) return 11; // NaN != NaN
        if (nan < 0) return 12;
        if (nan > 0) return 13;

        return 42;
    }
}
