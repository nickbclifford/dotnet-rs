using System;

public class Program {
    public static int Counter = 0;

    public static int Main() {
        // Very simple test: just increment a static counter
        // No synchronization - this tests basic multi-arena execution
        Counter++;
        return 0;
    }
}
