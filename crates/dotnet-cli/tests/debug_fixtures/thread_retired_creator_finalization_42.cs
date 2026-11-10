using System;
using System.Threading;

class Program
{
    static Thread target;

    static WeakReference Launch()
    {
        Thread creator = new Thread(() => { target = new Thread(() => { }); });
        creator.Start();
        creator.Join();
        Thread worker = target;
        target = null;
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
