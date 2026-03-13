namespace Statics;
using System;
using System.Threading;

public class MT_A {
    public static int x;
    static MT_A() {
        Program.Arrive();
        // This will block waiting for MT_B to finish initialization
        x = MT_B.y + 1;
    }
}

public class MT_B {
    public static int y;
    static MT_B() {
        Program.Arrive();
        // This will block waiting for MT_A to finish initialization
        y = MT_A.x + 1;
    }
}

public class Program {
    private static int counter = 0;
    private static int role_counter = 0;

    public static void Arrive() {
        Interlocked.Increment(ref counter);
        // Busy wait to ensure both threads have entered their respective .cctor
        while (Volatile.Read(ref counter) < 2) {
            // Busy wait
        }
    }

    public static int Main() {
        int role = Interlocked.Increment(ref role_counter) - 1;
        
        if (role == 0) {
            int val = MT_A.x;
            return (val == 2 || val == 1) ? 42 : 1;
        } else if (role == 1) {
            int val = MT_B.y;
            return (val == 2 || val == 1) ? 42 : 2;
        }
        
        return 42;
    }
}
