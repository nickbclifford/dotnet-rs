using System.Threading;

public static class Program
{
    public static int Main()
    {
        var ambient = new AsyncLocal<string>();
        ambient.Value = "safe-context";
        using var finished = new ManualResetEventSlim(false);
        int captured = 0;

        if (!ThreadPool.QueueUserWorkItem(_ =>
        {
            if (ambient.Value == "safe-context") Interlocked.Exchange(ref captured, 1);
            finished.Set();
        }, null)) return 1;

        bool parentUnchanged = ambient.Value == "safe-context";
        if (!finished.Wait(10000)) return 1;
        return captured == 1 && parentUnchanged && ambient.Value == "safe-context" ? 42 : 1;
    }
}
