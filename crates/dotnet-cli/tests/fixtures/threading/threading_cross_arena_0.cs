using System;
using System.Threading;

public class Program {
    public static int Counter = 0;
    public static object Lock = new object();

    public static int Main() {
        // Simple test: each thread increments a static counter using Monitor.Enter/Exit
        // This tests basic cross-arena coordination with synchronization

        lock (Lock) {
            Counter++;
        }

        return 0;
    }
}
