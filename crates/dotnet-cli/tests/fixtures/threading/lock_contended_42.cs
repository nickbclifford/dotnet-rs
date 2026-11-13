using System.Threading;

public static class Program
{
    public static int Main()
    {
        var gate = new Lock();
        using var held = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        var holder = new Thread(() =>
        {
            gate.Enter();
            held.Set();
            release.Wait();
            Thread.Sleep(200);
            gate.Exit();
        });
        holder.Start();
        held.Wait();
        release.Set();
        // The holder still owns the lock, so this takes TryEnterSlow and ThreadBlockingInfo.
        gate.Enter();
        gate.Exit();
        holder.Join();
        return 42;
    }
}
