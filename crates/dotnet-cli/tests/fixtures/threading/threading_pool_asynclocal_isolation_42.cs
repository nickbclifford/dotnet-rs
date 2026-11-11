using System.Threading;

public static class Program
{
    public static int Main()
    {
        var ambient = new AsyncLocal<string>();
        ambient.Value = "parent";
        using var finished = new ManualResetEventSlim(false);
        int parentCaptured = 0;
        int nextCallbackClear = 0;

        if (!ThreadPool.QueueUserWorkItem(_ =>
        {
            if (ambient.Value == "parent") Interlocked.Exchange(ref parentCaptured, 1);
            ambient.Value = "callback";
            if (!ThreadPool.UnsafeQueueUserWorkItem(_ =>
            {
                if (ambient.Value is null) Interlocked.Exchange(ref nextCallbackClear, 1);
                finished.Set();
            }, null))
            {
                finished.Set();
            }
        }, null)) return 1;

        bool parentUnchanged = ambient.Value == "parent";
        if (!finished.Wait(10000)) return 1;
        return parentCaptured == 1 && nextCallbackClear == 1 && parentUnchanged
            && ambient.Value == "parent" ? 42 : 1;
    }
}
