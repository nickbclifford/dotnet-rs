using System;
using System.Threading;

// Four threads race to lock each of many freshly allocated objects at the same moment.
// The first lock of an object creates its sync block; both threads must end up sharing it,
// so no thread may observe another inside the lock and Exit must never throw.
public class Program {
    const int Rounds = 3000;
    static readonly object[] objects = new object[Rounds];
    static readonly int[] inside = new int[Rounds];
    static int go, overlaps, lockErrors;

    static void Worker() {
        for (int i = 0; i < Rounds; i++) {
            while (Volatile.Read(ref go) <= i) { }
            try {
                lock (objects[i]) {
                    if (Interlocked.Increment(ref inside[i]) > 1) Interlocked.Increment(ref overlaps);
                    Thread.Sleep(0);
                    Interlocked.Decrement(ref inside[i]);
                }
            } catch (SynchronizationLockException) {
                Interlocked.Increment(ref lockErrors);
            }
        }
    }

    public static int Main() {
        for (int i = 0; i < Rounds; i++) objects[i] = new object();
        var threads = new Thread[4];
        for (int t = 0; t < threads.Length; t++) {
            threads[t] = new Thread(Worker);
            threads[t].Start();
        }
        for (int i = 1; i <= Rounds; i++) {
            Thread.Sleep(0);
            Volatile.Write(ref go, i);
        }
        foreach (var thread in threads) thread.Join();
        Console.WriteLine("overlaps " + overlaps + ", lock errors " + lockErrors);
        return overlaps == 0 && lockErrors == 0 ? 42 : 1;
    }
}
