using System;
using System.Threading;

class Program
{
    static void Work() { }
    static WeakReference Launch()
    {
        var worker = new Thread(Work);
        worker.Start();
        worker.Join();
        return new WeakReference(worker);
    }
    static int Main()
    {
        WeakReference weak = Launch();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return weak.IsAlive ? 1 : 42;
    }
}
