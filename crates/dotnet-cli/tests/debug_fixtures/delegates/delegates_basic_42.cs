// File: delegates/delegates_basic_42.cs
// Purpose: Basic delegate creation/invocation, multicast behavior, and lambdas
// Expected: Returns 42 on success

using System;

public class Program {
    private delegate int BinaryOp(int a, int b);

    public static int Main() {
        // Lambda delegate
        BinaryOp add = (a, b) => a + b;
        if (add(10, 32) != 42) return 1;

        // Method group conversion
        BinaryOp mul = Multiply;
        if (mul(6, 7) != 42) return 2;

        // Multicast delegate on Action-like signature using side effects
        int result = 0;
        Action<int> setter = x => result = x;
        setter += x => result += x;
        setter(21); // first sets to 21, second adds 21 => 42
        if (result != 42) return 3;

        return 42;
    }

    private static int Multiply(int a, int b) => a * b;
}
