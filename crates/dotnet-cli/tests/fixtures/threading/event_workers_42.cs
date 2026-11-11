using System;
using System.Threading;

static class Program
{
    static int ready, completed;
    static void MainWait(AutoResetEvent gate, AutoResetEvent done)
    {
        Interlocked.Increment(ref ready);
        gate.WaitOne();
        Interlocked.Increment(ref completed);
        done.Set();
    }
    static int Main()
    {
        using var auto = new AutoResetEvent(false);
        using var done = new AutoResetEvent(false);
        var a = new Thread(() => MainWait(auto, done));
        var b = new Thread(() => MainWait(auto, done));
        a.Start(); b.Start();
        while (Volatile.Read(ref ready) != 2) Thread.Yield();
        auto.Set();
        done.WaitOne();
        if (Volatile.Read(ref completed) != 1) return 1;
        auto.Set();
        a.Join(); b.Join();
        if (completed != 2) return 2;

        ready = completed = 0;
        using var manual = new ManualResetEvent(false);
        var c = new Thread(() => { Interlocked.Increment(ref ready); manual.WaitOne(); Interlocked.Increment(ref completed); });
        var d = new Thread(() => { Interlocked.Increment(ref ready); manual.WaitOne(); Interlocked.Increment(ref completed); });
        c.Start(); d.Start();
        while (Volatile.Read(ref ready) != 2) Thread.Yield();
        manual.Set();
        c.Join(); d.Join();
        if (completed != 2 || !manual.WaitOne(0)) return 3;
        manual.Reset();
        if (manual.WaitOne(0)) return 4;
        return 42;
    }
}
