using System.Threading;

public static class Program
{
    static readonly object gate = new object();
    static bool enterTaken, timedTaken;
    static volatile bool attempting, done;
    static int result;

    static void EnterWorker()
    {
        attempting = true;
        Monitor.Enter(gate, ref enterTaken);
        if (enterTaken) { result += 20; Monitor.Exit(gate); }
        done = true;
    }

    static void TimedWorker()
    {
        attempting = true;
        Monitor.TryEnter(gate, 300, ref timedTaken);
        if (!timedTaken) result += 22;
        else Monitor.Exit(gate);
        done = true;
    }

    public static int Main()
    {
        Monitor.Enter(gate);
        var worker = new Thread(EnterWorker);
        worker.Start();
        while (!attempting) Thread.Sleep(1);
        Thread.Sleep(40);
        GC.Collect();
        if (done || enterTaken) return 1;
        Monitor.Exit(gate);
        worker.Join();
        if (!done || !enterTaken) return 2;

        attempting = false;
        done = false;
        Monitor.Enter(gate);
        worker = new Thread(TimedWorker);
        worker.Start();
        while (!attempting) Thread.Sleep(1);
        Thread.Sleep(40);
        GC.Collect();
        while (!done) Thread.Sleep(1);
        Monitor.Exit(gate);
        worker.Join();
        return result == 42 && !timedTaken ? 42 : 3;
    }
}
