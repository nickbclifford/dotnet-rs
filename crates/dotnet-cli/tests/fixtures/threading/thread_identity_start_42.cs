using System;
using System.Threading;

public class Program {
    private static Thread worker;
    private static int workerId;
    private static int observedArgument;
    private static bool identity;
    private static volatile bool gate;

    public static int Main() {
        int entryId = Thread.CurrentThread.ManagedThreadId;
        int captured = 0;
        worker = new Thread((object value) => {
            identity = ReferenceEquals(Thread.CurrentThread, worker);
            workerId = Thread.CurrentThread.ManagedThreadId;
            observedArgument = (int)value;
            GC.Collect();
            while (!gate) Thread.Yield();
            captured = 42;
        });
        int expectedId = worker.ManagedThreadId;
        worker.Start(17);
        if (worker.Join(30)) return 1;
        gate = true;
        if (!worker.Join(5000) || !worker.Join(0)) return 2;
        if (!identity || workerId != expectedId || entryId == workerId) return 3;
        if (entryId != Environment.CurrentManagedThreadId || observedArgument != 17 || captured != 42) return 4;
        return 42;
    }
}
