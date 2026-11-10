using System;
using System.Threading;

public class Program {
    public static int Main() {
        Thread.Yield();
        Thread.SpinWait(100);
        Thread.SpinWait(-1);
        Thread.Sleep(0);
        Thread.Sleep(20);
        Thread.Sleep(TimeSpan.FromMilliseconds(20));
        try { Thread.Sleep(-2); return 1; }
        catch (ArgumentOutOfRangeException) { }
        try { Thread.Sleep(TimeSpan.FromMilliseconds(-2)); return 2; }
        catch (ArgumentOutOfRangeException) { }
        return 42;
    }
}
