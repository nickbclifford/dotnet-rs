using System.Threading;

public static class Program
{
    static object gate = new object();
    static volatile bool held;
    static volatile bool release;
    static bool workerSawEntered = true;

    static void Worker()
    {
        workerSawEntered = Monitor.IsEntered(gate);
        lock (gate)
        {
            held = true;
            while (!release) Thread.Sleep(1);
        }
    }

    public static int Main()
    {
        if (Monitor.IsEntered(gate)) return 1;
        lock (gate)
        {
            if (!Monitor.IsEntered(gate)) return 2;
            lock (gate) { if (!Monitor.IsEntered(gate)) return 3; }
            if (!Monitor.IsEntered(gate)) return 4;
        }
        if (Monitor.IsEntered(gate)) return 5;

        var t = new Thread(Worker);
        t.Start();
        while (!held) Thread.Sleep(1);
        // Another thread owns the lock; this thread must not report ownership.
        if (Monitor.IsEntered(gate)) return 6;
        release = true;
        t.Join();
        if (workerSawEntered) return 7;
        return 42;
    }
}
