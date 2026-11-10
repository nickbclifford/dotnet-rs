using System;
using System.Threading;

public class Program {
    static int ready;
    static void Worker() {
        Volatile.Write(ref ready, 1);
        Thread.Sleep(3000);
    }
    public static int Main() {
        var worker = new Thread(Worker);
        worker.Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        Thread.Sleep(50);
        var start = Environment.TickCount64;
        GC.Collect();
        long elapsed = Environment.TickCount64 - start;
        worker.Join();
        return elapsed < 1500 ? 42 : 1;
    }
}
