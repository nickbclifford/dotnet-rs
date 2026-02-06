// File: arithmetic/math_intrinsics_42.cs
// Purpose: Test Math intrinsics
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        if (Math.Min(10, 20) != 10) return 1;
        if (Math.Max(10, 20) != 20) return 2;
        if (Math.Abs(-10) != 10) return 3;
        
        if (Math.Min(10.5, 20.5) != 10.5) return 4;
        if (Math.Max(10.5, 20.5) != 20.5) return 5;
        if (Math.Abs(-10.5) != 10.5) return 6;
        
        if (Math.Sqrt(16.0) != 4.0) return 7;
        if (Math.Pow(2.0, 3.0) != 8.0) return 8;

        return 42;
    }
}
