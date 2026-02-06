using System;

public class Program {
    public static int Main() {
        int count = 0;
        
        // 1. Success case
        try {
            checked {
                int a = 10;
                int b = 32;
                if (a + b == 42) count++;
            }
        } catch (OverflowException) {
            return 1;
        }

        // 2. Overflow case (Signed)
        try {
            checked {
                int a = int.MaxValue;
                int b = 1;
                int c = a + b;
                return 2; // Should not reach here
            }
        } catch (OverflowException) {
            count++;
        }

        // 3. Underflow case (Signed)
        try {
            checked {
                int a = int.MinValue;
                int b = -1;
                int c = a + b;
                return 3; // Should not reach here
            }
        } catch (OverflowException) {
            count++;
        }

        if (count == 3) return 42;
        return count;
    }
}
