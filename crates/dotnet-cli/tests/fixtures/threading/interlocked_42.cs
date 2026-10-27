using System;
using System.Threading;

public class Holder { public object Value; }

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
        object obj3 = new object();
        prevObj = Interlocked.CompareExchange(ref obj, obj3, prevObj);
        if (prevObj != obj2 || obj != obj2) return 7;
        prevObj = Interlocked.Exchange(ref obj, obj3);
        if (prevObj != obj2 || obj != obj3) return 8;
        prevObj = Interlocked.Exchange(ref obj, null);
        if (prevObj != obj3 || obj != null) return 9;
        object local = obj2;
        prevObj = Interlocked.CompareExchange(ref local, obj3, obj2);
        if (prevObj != obj2 || local != obj3) return 10;
        prevObj = Interlocked.CompareExchange(ref local, obj2, obj2);
        if (prevObj != obj3 || local != obj3) return 11;
        prevObj = Interlocked.Exchange(ref local, null);
        if (prevObj != obj3 || local != null) return 12;
        Holder holder = new Holder();
        holder.Value = obj2;
        prevObj = Interlocked.CompareExchange(ref holder.Value, obj3, obj2);
        if (prevObj != obj2 || holder.Value != obj3) return 13;
        prevObj = Interlocked.Exchange(ref holder.Value, null);
        if (prevObj != obj3 || holder.Value != null) return 14;

        return 42;
    }
}
