using System.Threading;

public static class Program
{
    public static int Main()
    {
        using var fired = new ManualResetEventSlim();
        int ranOnPoolThread = 0;
        using var timer = new Timer(_ =>
        {
            if (Thread.CurrentThread.IsThreadPoolThread)
                Interlocked.Exchange(ref ranOnPoolThread, 1);
            fired.Set();
        }, null, 0, Timeout.Infinite);

        fired.Wait();
        return Volatile.Read(ref ranOnPoolThread) == 1 ? 42 : 1;
    }
}
