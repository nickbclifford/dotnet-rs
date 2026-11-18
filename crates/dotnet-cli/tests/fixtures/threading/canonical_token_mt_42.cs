using System;
using System.Runtime.CompilerServices;
using System.Threading;

class WorkerType { }
class RaceType { }
class SynchronizedType
{
    [MethodImpl(MethodImplOptions.Synchronized)]
    public static void Enter() { }
}
static class Program
{
    static Type? first, left, right;
    static volatile bool go;
    static int ready;
    static void Race(bool a)
    {
        Interlocked.Increment(ref ready);
        while (!go) Thread.Sleep(0);
        var value = typeof(RaceType);
        if (a) left = value; else right = value;
    }
    static int Main()
    {
        var worker = new Thread(() => { first = typeof(WorkerType); SynchronizedType.Enter(); });
        worker.Start(); worker.Join();
        bool ok = ReferenceEquals(first, typeof(WorkerType));
        var weak = new WeakReference(first);
        var keys = new ConditionalWeakTable<Type, object>();
        keys.Add(first!, new object());
        first = null;
        var a = new Thread(() => Race(true));
        var b = new Thread(() => Race(false));
        a.Start(); b.Start();
        while (Volatile.Read(ref ready) != 2) Thread.Sleep(0);
        go = true;
        a.Join(); b.Join();
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        ok = ok && ReferenceEquals(left, right) && ReferenceEquals(left, typeof(RaceType));
        ok = ok && ReferenceEquals(weak.Target, typeof(WorkerType)) && keys.TryGetValue(typeof(WorkerType), out _);
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
