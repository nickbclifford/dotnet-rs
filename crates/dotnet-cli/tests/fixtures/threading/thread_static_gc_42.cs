using System;
using System.Threading;

class Program
{
    [ThreadStatic] static Finalizable Value;
    static int finalized;
    class Finalizable { ~Finalizable() { Interlocked.Increment(ref finalized); } }
    static void Worker() { Value = new Finalizable(); }
    static int Main()
    {
        var worker = new Thread(Worker);
        worker.Start();
        worker.Join();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        bool ok = finalized == 1;
        Console.WriteLine($"thread-static-finalized={ok}");
        return ok ? 42 : 1;
    }
}
