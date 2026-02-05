using System;
using System.Threading;

public class Program {
    static int val = 10;
    static long val64 = 100;
    static object obj = new object();

    public static int Main() {
        int prev = Interlocked.CompareExchange(ref val, 20, 10);
        if (prev != 10 || val != 20) return 1;

        prev = Interlocked.CompareExchange(ref val, 30, 10);
        if (prev != 20 || val != 20) return 2;

        prev = Interlocked.Exchange(ref val, 42);
        if (prev != 20 || val != 42) return 3;

        long prev64 = Interlocked.CompareExchange(ref val64, 200, 100);
        if (prev64 != 100 || val64 != 200) return 4;

        prev64 = Interlocked.Exchange(ref val64, 42);
        if (prev64 != 200 || val64 != 42) return 5;

        object obj2 = new object();
        object prevObj = Interlocked.CompareExchange(ref obj, obj2, obj);
        if (prevObj == null || obj != obj2) return 6;

        return 42;
    }
}
