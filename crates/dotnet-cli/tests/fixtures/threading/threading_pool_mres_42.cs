using System.Threading;

public static class Program
{
    public static int Main()
    {
        using var ready = new ManualResetEventSlim(false);
        int result = 0;
        if (!ThreadPool.QueueUserWorkItem(_ => { result = 42; ready.Set(); })) return 1;
        ready.Wait();
        return result;
    }
}
