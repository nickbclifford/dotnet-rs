using System;
using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        ThreadPool.GetMinThreads(out int min, out int minIo);
        ThreadPool.GetMaxThreads(out int max, out int maxIo);
        if (min < 1 || minIo < 1 || max < min || maxIo < minIo) return 1;
        if (ThreadPool.SetMinThreads(-1, 1) || ThreadPool.SetMaxThreads(0, 1)) return 2;
        if (!ThreadPool.SetMinThreads(0, 0)) return 3;
        ThreadPool.GetMinThreads(out int newMin, out int newMinIo);
        if (newMin != 1 || newMinIo != 1) return 4;
        if (!ThreadPool.SetMaxThreads(32, 32)) return 5;
        ThreadPool.GetMaxThreads(out int newMax, out int newMaxIo);
        if (newMax != 32 || newMaxIo != 32) return 6;
        long before = ThreadPool.CompletedWorkItemCount;
        object gate = new object();
        int called = 0;
        WaitCallback callback = _ => { lock (gate) { called++; Monitor.PulseAll(gate); } };
        lock (gate)
        {
            ThreadPool.QueueUserWorkItem(callback);
            ThreadPool.QueueUserWorkItem(callback);
            while (called < 2) Monitor.Wait(gate);
        }
        while (ThreadPool.CompletedWorkItemCount < before + 2) Thread.Yield();
        GC.Collect();
        if (Task.Run(() => 42).Result != 42) return 8;
        return 42;
    }
}
