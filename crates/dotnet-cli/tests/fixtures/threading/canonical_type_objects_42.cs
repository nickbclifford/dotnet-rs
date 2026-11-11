using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;

class WorkerOwned { }
class MainOwned { }
class RaceType { }

static class Program
{
    static Type? workerType, mainType;
    static object? workerObject;
    static object? mainObject;
    static Dictionary<Type, int>? types;
    static Type? raceA, raceB;
    static volatile bool raceStart;
    static bool workerOk;
    static int inLock, lockCount, inSync, syncCount;
    static bool lockOverlap, syncOverlap;

    static void BumpLock()
    {
        lock (typeof(WorkerOwned))
        {
            if (++inLock != 1) lockOverlap = true;
            int n = lockCount;
            Thread.Sleep(0);
            lockCount = n + 1;
            --inLock;
        }
    }

    [MethodImpl(MethodImplOptions.Synchronized)]
    static void BumpSync()
    {
        if (++inSync != 1) syncOverlap = true;
        int n = syncCount;
        Thread.Sleep(0);
        syncCount = n + 1;
        --inSync;
    }

    static int Main()
    {
        mainType = typeof(MainOwned);
        mainObject = new MainOwned();
        var creator = new Thread(() =>
        {
            workerType = typeof(WorkerOwned);
            workerObject = new WorkerOwned();
            types = new Dictionary<Type, int>();
            types.Add(workerType, 42);
            workerOk = mainObject!.GetType() == typeof(MainOwned)
                && Object.ReferenceEquals(mainType, typeof(MainOwned));
        });
        creator.Start();
        creator.Join();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        bool ok = workerOk && Object.ReferenceEquals(workerType, typeof(WorkerOwned))
            && workerObject!.GetType() == typeof(WorkerOwned)
            && types![typeof(WorkerOwned)] == 42;

        var a = new Thread(() => { while (!raceStart) Thread.Sleep(0); raceA = typeof(RaceType); });
        var b = new Thread(() => { while (!raceStart) Thread.Sleep(0); raceB = typeof(RaceType); });
        a.Start(); b.Start(); raceStart = true; a.Join(); b.Join();
        ok = ok && Object.ReferenceEquals(raceA, raceB) && Object.ReferenceEquals(raceA, typeof(RaceType));
        GC.Collect();
        ok = ok && Object.ReferenceEquals(workerType, typeof(WorkerOwned));

        var contender = new Thread(() => { for (int i = 0; i < 100; i++) { BumpLock(); BumpSync(); } });
        contender.Start();
        for (int i = 0; i < 100; i++) { BumpLock(); BumpSync(); }
        contender.Join();
        ok = ok && !lockOverlap && lockCount == 200 && !syncOverlap && syncCount == 200;
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
