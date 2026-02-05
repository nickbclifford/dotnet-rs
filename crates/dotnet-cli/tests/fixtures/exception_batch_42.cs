using System;

public class Program {
    public static int Main() {
        int x = 0;
        try {
            DoThrow();
            x = 1; // Should NOT be executed
            x = 2;
            x = 3;
            // Add many instructions to fill the batch
            x = 4; x = 5; x = 6; x = 7; x = 8; x = 9; x = 10;
        } catch (Exception) {
            if (x == 0) return 42;
            return x;
        }
        return 0;
    }

    static void DoThrow() {
        throw new Exception();
    }
}
