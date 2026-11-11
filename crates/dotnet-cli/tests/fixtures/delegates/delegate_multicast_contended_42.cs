using System;
using System.Threading;

public static class Program {
    static readonly object gate = new object();
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
    static void Before(object _) { attempting = true; }
    static void After(object _) { value += 2; }
    public static int Main() {
        Thread owner = new Thread(Owner);
        owner.Start();
        while (!held) Thread.Sleep(1);
        Action<object> chain = Before;
        chain += Monitor.Enter;
        chain += After;
        chain(gate);
        Monitor.Exit(gate);
        owner.Join();
        Console.WriteLine(value);
        return value == 42 ? 42 : 1;
    }
}
