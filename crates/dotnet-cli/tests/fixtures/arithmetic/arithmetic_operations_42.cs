// File: arithmetic/arithmetic_operations_42.cs
// Purpose: Comprehensive arithmetic tests
// Expected: Returns 42 on success

using System;

public class Program {
    // Calls keep operands out of C# constant folding so Main contains these IL operations.
    static int Ten() => 10;
    static int Three() => 3;
    static int NegativeTen() => -10;
    static int One() => 1;
    static int Eight() => 8;
    static int NegativeEight() => -8;
    static int Two() => 2;
    static int Fifteen() => 0x0F;
    static int F0() => 0xF0;
    static int FF() => 0xFF;
    static int Four() => 4;

    public static int Main() {
        int ten = Ten();
        int three = Three();
        int negativeTen = NegativeTen();
        int one = One();
        int eight = Eight();
        int negativeEight = NegativeEight();
        int two = Two();
        int fifteen = Fifteen();
        int f0 = F0();
        int ff = FF();
        int four = Four();

        // Division
        if (ten / three != 3) return 1;
        if (negativeTen / three != -3) return 2;
        
        // Remainder
        if (ten % three != 1) return 3;
        if (negativeTen % three != -1) return 4;
        
        // Shift operations
        if ((one << three) != 8) return 5;
        if ((eight >> two) != 2) return 6;
        if ((negativeEight >> two) != -2) return 7; // Arithmetic shift
        
        // Bitwise
        if ((ff & fifteen) != 0x0F) return 8;
        if ((f0 | fifteen) != 0xFF) return 9;
        if ((ff ^ fifteen) != 0xF0) return 10;
        
        // Multiplication and addition
        if (two * three + four != 10) return 11;
        if (ten - two * three != 4) return 12;

        return 42;
    }
}
