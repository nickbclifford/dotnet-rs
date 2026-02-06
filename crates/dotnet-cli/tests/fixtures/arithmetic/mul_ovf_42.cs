using System;

public class Program {
    public static int Main() {
        int count = 0;

        // 1. Success case
        try {
            checked {
                int a = 6;
                int b = 7;
                if (a * b == 42) count++;
            }
        } catch (OverflowException) {
            return 1;
        }

        // 2. Overflow case
        try {
            checked {
                int a = int.MaxValue / 2 + 1;
                int b = 2;
                int c = a * b;
                return 2;
            }
        } catch (OverflowException) {
            count++;
        }

        if (count == 2) return 42;
        return count;
    }
}
