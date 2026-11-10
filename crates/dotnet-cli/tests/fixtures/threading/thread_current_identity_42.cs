using System;
using System.Threading;

public class Program {
    public static int Main() {
        Thread current = Thread.CurrentThread;
        int id = current.ManagedThreadId;
        GC.Collect();
        if (current != Thread.CurrentThread || id != Environment.CurrentManagedThreadId) return 1;
        if (current.Join(0)) return 2;
        try { current.Join(-2); return 3; }
        catch (ArgumentOutOfRangeException) { }
        return 42;
    }
}
