using System;
using System.Threading;

public class Program {
    private static volatile bool release;
    private static bool crossJoin;
    private static int first;
    private static int second;

    public static int Main() {
        int captured = 21;
        ThreadStart start = () => { while (!release) Thread.Yield(); first = captured; };
        start += () => { second = captured; };
        Thread worker = new Thread(start);
        int id = worker.ManagedThreadId;
        worker.Start();
        Thread observer = new Thread((object arg) => {
            Thread other = (Thread)arg;
            bool valid = other.ManagedThreadId == id && !other.Join(25);
            GC.Collect();
            release = true;
            crossJoin = other.Join(5000) && valid;
        });
        observer.Start(worker);
        if (!observer.Join(5000)) return 1;
        if (!crossJoin || !worker.Join(0) || first != 21 || second != 21) return 2;
        return 42;
    }
}
