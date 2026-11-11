using System.Threading;

public static class Program
{
    public static int Main()
    {
        var ambient = new AsyncLocal<string>();
        ambient.Value = "must-not-flow";
        using var finished = new ManualResetEventSlim(false);
        int remainedClear = 0;

        if (!ThreadPool.UnsafeQueueUserWorkItem(_ =>
        {
            if (ambient.Value is null) Interlocked.Exchange(ref remainedClear, 1);
            finished.Set();
        }, null)) return 1;

        bool parentUnchanged = ambient.Value == "must-not-flow";
        if (!finished.Wait(10000)) return 1;
        return remainedClear == 1 && parentUnchanged && ambient.Value == "must-not-flow" ? 42 : 1;
    }
}
