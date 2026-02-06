// File: exceptions/nested_exceptions_42.cs
// Purpose: Nested try/catch/finally behavior without console output
// Expected: Returns 42 on success

using System;

public class Program {
    public static int Main() {
        int state = 0;

        try {
            state += 1; // 1
            try {
                state += 2; // 3
                ThrowOuter();
                return 1; // should not reach
            } catch (InvalidOperationException) {
                state += 4; // 7
            } finally {
                state += 8; // 15
            }

            // Outer continues after inner handled
            if (state != 15) return 2;

            try {
                state += 16; // 31
                ThrowInner();
                return 3; // should not reach
            } catch (ArgumentException) {
                state += 32; // 63
            } finally {
                state += 64; // 127
            }

            if (state != 127) return 4;
        } catch (Exception) {
            return 5; // no exception should escape
        } finally {
            state += 128; // 255
        }

        if (state != 255) return 6;
        return 42;
    }

    static void ThrowOuter() {
        throw new InvalidOperationException();
    }

    static void ThrowInner() {
        throw new ArgumentException();
    }
}
