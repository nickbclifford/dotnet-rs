using System;
using System.Threading;

public class Program {
    public static int Main() {
        int val = 10;
        if (Volatile.Read(ref val) != 10) return 1;
        Volatile.Write(ref val, 42);
        if (Volatile.Read(ref val) != 42) return 5;

        long val64 = 0;
        Volatile.Write(ref val64, 42);
        if (Volatile.Read(ref val64) != 42) return 2;

        double valD = 0;
        Volatile.Write(ref valD, 42.0);
        if (Volatile.Read(ref valD) != 42.0) return 3;

        object obj = new object();
        Volatile.Write(ref obj, null);
        if (Volatile.Read(ref obj) != null) return 4;

        return 42;
    }
}
