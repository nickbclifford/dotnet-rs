// File: arithmetic/sub_ovf_42.cs
// Purpose: Test checked subtraction overflow behavior for signed/unsigned widths
// Tests: sub.ovf, sub.ovf.un instructions via C# checked contexts
// Expected: Returns 42 on success
// Related: ECMA-335 III.3.1 (Arithmetic instructions)

using System;

public class Program {
    public static int Main() {
        // Signed 32-bit: MinValue - 1 should throw in checked context
        try {
            int a = int.MinValue;
            int b = checked(a - 1);
            return 1; // should not reach
        } catch (OverflowException) {
            // expected
        }

        // Unsigned 32-bit: 0u - 1u should throw in checked context
        try {
            uint ua = 0u;
            uint ub = checked(ua - 1u);
            return 2; // should not reach
        } catch (OverflowException) {
            // expected
        }

        // Signed 64-bit: MinValue - 1 should throw in checked context
        try {
            long la = long.MinValue;
            long lb = checked(la - 1L);
            return 3; // should not reach
        } catch (OverflowException) {
            // expected
        }

        // Non-overflowing cases should compute normally under checked
        int x = 100;
        int y = 58;
        int r = checked(x - y);
        if (r != 42) return 4;

        long xl = 100000000000L;
        long yl = 99999999958L;
        long rl = checked(xl - yl);
        if (rl != 42L) return 5;

        // Mixed signed/unsigned sanity (promotions are well-defined in C#)
        uint u2 = 100u;
        uint u3 = 58u;
        uint ur = checked(u2 - u3);
        if (ur != 42u) return 6;

        return 42;
    }
}
