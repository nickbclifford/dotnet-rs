using System;
using System.Threading;

public static class Slots<T>
{
    [ThreadStatic] public static int Value;
    [ThreadStatic] public static object Root;
    public static int Shared;
    static Slots() { Value = 7; }
}

public sealed class Holder { public int Value; }

public static class Program
{
    static int Exchange(ref int value, int next) => Interlocked.Exchange(ref value, next);

    public static int Main()
    {
        if (Slots<int>.Value != 7 || Slots<string>.Value != 7) return 1;
        Slots<int>.Value = 11;
        Slots<string>.Value = 12;
        Slots<int>.Shared = 100;
        object gate = new object();
        bool ready = false, done = false;
        int result = 0;
        Thread worker = new Thread(() => {
            if (Slots<int>.Value != 0 || Slots<string>.Value != 0 || Slots<int>.Shared != 100) result = 2;
            Slots<int>.Root = new Holder { Value = 42 };
            if (Exchange(ref Slots<int>.Value, 31) != 0 || Exchange(ref Slots<string>.Value, 32) != 0) result = 3;
            lock (gate) {
                ready = true;
                Monitor.PulseAll(gate);
                while (!done) Monitor.Wait(gate);
            }
            if (((Holder)Slots<int>.Root).Value != 42 || Slots<int>.Value != 31 || Slots<string>.Value != 32) result = 4;
        });
        lock (gate) {
            worker.Start();
            while (!ready) Monitor.Wait(gate);
            GC.Collect();
            if (Slots<int>.Value != 11 || Slots<string>.Value != 12 || Slots<int>.Root != null) result = 5;
            done = true;
            Monitor.PulseAll(gate);
        }
        worker.Join();
        Console.WriteLine($"thread-static={result == 0}");
        return result == 0 ? 42 : result;
    }
}
