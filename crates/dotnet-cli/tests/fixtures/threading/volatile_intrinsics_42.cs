using System;
using System.Threading;

public class Holder { public object Value; }

public class Program {
    static object shared;

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
        object another = new object();
        Volatile.Write(ref obj, another);
        if (Volatile.Read(ref obj) != another) return 6;
        Volatile.Write(ref shared, another);
        if (Volatile.Read(ref shared) != another) return 7;
        Volatile.Write(ref shared, null);
        if (Volatile.Read(ref shared) != null) return 8;
        Holder holder = new Holder();
        Volatile.Write(ref holder.Value, another);
        if (Volatile.Read(ref holder.Value) != another) return 9;
        Volatile.Write(ref holder.Value, null);
        if (Volatile.Read(ref holder.Value) != null) return 10;

        return 42;
    }
}
