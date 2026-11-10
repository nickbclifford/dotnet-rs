using System;
using System.Threading;

public static class Program {
    static object gate = new object();
    static volatile bool held, attempting;
    static int value;
    static void Owner() {
        lock (gate) {
            held = true;
            while (!attempting) Thread.Sleep(1);
            Thread.Sleep(40);
            value = 40;
        }
    }
    public static int Main() {
        object local = gate;
        Thread owner = new Thread(Owner);
        owner.Start();
        while (!held) Thread.Sleep(1);
        Action<object> enter = Monitor.Enter;
        attempting = true;
        enter(local);
        try { value += 2; } finally { Monitor.Exit(local); }
        owner.Join();
        return value == 42 ? 42 : 1;
    }
}
