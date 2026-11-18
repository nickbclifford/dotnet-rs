using System;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading;

class AssemblySubject { }

static class Program
{
    static volatile bool start;
    static int ready;
    static Assembly? first, second;
    static readonly ConditionalWeakTable<Assembly, object> keys = new();

    static void MainWorker(bool a)
    {
        Interlocked.Increment(ref ready);
        while (!start) Thread.Sleep(0);
        var assembly = typeof(AssemblySubject).Assembly;
        if (a) first = assembly; else second = assembly;
    }

    static int Main()
    {
        var a = new Thread(() => MainWorker(true));
        var b = new Thread(() => MainWorker(false));
        a.Start(); b.Start();
        while (Volatile.Read(ref ready) != 2) Thread.Sleep(0);
        start = true;
        a.Join(); b.Join();
        bool same = first != null && ReferenceEquals(first, second)
            && ReferenceEquals(first, typeof(AssemblySubject).Assembly);
        bool reverse = first!.GetName().Name != null;
        keys.Add(first, new object());
        var weak = new WeakReference(first);
        first = second = null;
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        var winner = typeof(AssemblySubject).Assembly;
        bool alive = ReferenceEquals(weak.Target, winner) && keys.TryGetValue(winner, out _);
        Console.WriteLine("same: " + same);
        Console.WriteLine("reverse: " + reverse);
        Console.WriteLine("alive: " + alive);
        return same && reverse && alive ? 42 : 1;
    }
}
