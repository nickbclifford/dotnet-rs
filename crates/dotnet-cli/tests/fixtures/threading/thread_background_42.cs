using System;
using System.Threading;

public static class Program {
    static volatile bool spinning;
    static volatile bool sleeping;
    static void Spin() {
        spinning = true;
        while (true) Thread.Yield();
    }
    static void Sleep() {
        sleeping = true;
        Thread.Sleep(Timeout.Infinite);
    }
    static void Foreground() {
        Thread.Sleep(100);
        Console.WriteLine("foreground completed");
    }
    public static int Main() {
        var spin = new Thread(Spin);
        if (spin.IsBackground || Thread.CurrentThread.IsBackground) return 1;
        spin.IsBackground = true;
        if (!spin.IsBackground) return 2;
        spin.Start();
        var sleep = new Thread(Sleep);
        sleep.Start();
        while (!spinning || !sleeping) Thread.Yield();
        sleep.IsBackground = true;
        if (!sleep.IsBackground) return 3;
        var foreground = new Thread(Foreground);
        foreground.IsBackground = true;
        foreground.Start();
        foreground.IsBackground = false;
        if (foreground.IsBackground) return 4;
        return 42;
    }
}
