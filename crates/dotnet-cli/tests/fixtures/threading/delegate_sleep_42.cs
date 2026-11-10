using System;
using System.Threading;

public static class Program {
    public static int Main() {
        Action<int> sleep = Thread.Sleep;
        sleep(0);
        sleep(20);
        try { sleep(-2); return 1; }
        catch (ArgumentOutOfRangeException) { }
        return 42;
    }
}
