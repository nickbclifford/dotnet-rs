using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;

class Program
{
    static ThreadLocal<object> untracked = new ThreadLocal<object>();
    static ThreadLocal<object> tracked = new ThreadLocal<object>(true);
    static WeakReference weakUntracked, weakTracked;
    static IList<object> snapshot;
    static void Worker()
    {
        untracked.Value = new object();
        tracked.Value = new object();
        weakUntracked = new WeakReference(untracked.Value);
        weakTracked = new WeakReference(tracked.Value);
    }
    static void Collect() { GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect(); }
    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool CheckSnapshot()
    {
        snapshot = tracked.Values;
        return snapshot.Count == 1 && Object.ReferenceEquals(snapshot[0], weakTracked.Target);
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool SnapshotIsLive() { return weakTracked.IsAlive && snapshot.Count == 1; }
    static int Main()
    {
        var worker = new Thread(Worker);
        worker.Start(); worker.Join();
        Collect();
        bool released = !weakUntracked.IsAlive;
        bool retained = weakTracked.IsAlive && CheckSnapshot();
        tracked.Dispose();
        untracked.Dispose();
        Collect();
        bool snapshotRetained = SnapshotIsLive();
        snapshot = null;
        Collect();
        bool disposed = !weakTracked.IsAlive;
        Console.WriteLine($"thread-local-released={released} retained={retained} snapshot={snapshotRetained} disposed={disposed}");
        return released && retained && snapshotRetained && disposed ? 42 : 1;
    }
}
