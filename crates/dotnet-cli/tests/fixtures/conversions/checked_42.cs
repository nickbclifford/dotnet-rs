// File: conversions/checked_42.cs
// Purpose: Test checked conversion instructions (conv.ovf.*)
// Tests: conv.ovf.i1, conv.ovf.u1, etc.
// Expected: Returns 42 on success, throws OverflowException on overflow

using System;

public class Program {
    public static int Main() {
        // Test successful conversions
        if (TestI1(127) != 127) return 1;
        if (TestU1(255) != 255) return 2;
        if (TestI2(32767) != 32767) return 3;
        if (TestU2(65535) != 65535) return 4;

        // Test overflow detection
        try {
            TestI1(128);
            return 5;
        } catch (OverflowException) {
            // Expected
        }

        try {
            TestU1(256);
            return 6;
        } catch (OverflowException) {
            // Expected
        }

        try {
            TestI1(-129);
            return 7;
        } catch (OverflowException) {
            // Expected
        }

        try {
            TestU1(-1);
            return 8;
        } catch (OverflowException) {
            // Expected
        }

        // Test conversion from float to int with overflow
        try {
            TestI1Float(128.0f);
            return 9;
        } catch (OverflowException) {
            // Expected
        }

        return 42;
    }

    static sbyte TestI1(int x) {
        checked {
            return (sbyte)x;
        }
    }

    static byte TestU1(int x) {
        checked {
            return (byte)x;
        }
    }

    static short TestI2(int x) {
        checked {
            return (short)x;
        }
    }

    static ushort TestU2(int x) {
        checked {
            return (ushort)x;
        }
    }

    static sbyte TestI1Float(float x) {
        checked {
            return (sbyte)x;
        }
    }
}
