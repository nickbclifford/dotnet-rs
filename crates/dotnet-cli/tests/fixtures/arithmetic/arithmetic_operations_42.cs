// File: arithmetic/arithmetic_operations_42.cs
// Purpose: Comprehensive arithmetic tests
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        // Division
        if (10 / 3 != 3) return 1;
        if (-10 / 3 != -3) return 2;
        
        // Remainder
        if (10 % 3 != 1) return 3;
        if (-10 % 3 != -1) return 4;
        
        // Shift operations
        if ((1 << 3) != 8) return 5;
        if ((8 >> 2) != 2) return 6;
        if ((-8 >> 2) != -2) return 7; // Arithmetic shift
        
        // Bitwise
        if ((0xFF & 0x0F) != 0x0F) return 8;
        if ((0xF0 | 0x0F) != 0xFF) return 9;
        if ((0xFF ^ 0x0F) != 0xF0) return 10;
        
        // Multiplication and addition
        if (2 * 3 + 4 != 10) return 11;
        if (10 - 2 * 3 != 4) return 12;

        return 42;
    }
}
