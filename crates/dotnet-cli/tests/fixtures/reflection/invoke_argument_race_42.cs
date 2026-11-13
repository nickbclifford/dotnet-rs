using System;
using System.Diagnostics;
using System.Reflection;
using System.Threading;

static class Program
{
    private static readonly object[] Arguments = { 1 };
    private static volatile bool stopRacer;
    // Volatile updates leave a scheduling window between successive array replacements.
    private static volatile int churn;
    private static readonly ManualResetEventSlim racerReady = new(false);
    private const long TimeoutMs = 30_000;

    private static void Target(ref int value)
    {
        _ = value;
    }

    private static void ReplaceArgument()
    {
        racerReady.Set();
        // Runs until main observes the rejection (or gives up): no cycle cap, so CPU
        // starvation cannot end the race early.
        while (!stopRacer)
        {
            Arguments[0] = 1L;
            for (var spin = 0; spin < 10; spin++)
                churn++;
            Arguments[0] = 1;
            for (var spin = 0; spin < 10; spin++)
                churn++;
        }
    }

    public static int Main()
    {
        var method = typeof(Program).GetMethod(
            nameof(Target), BindingFlags.Static | BindingFlags.NonPublic)!;
        var racer = new Thread(ReplaceArgument);
        racer.Start();
        racerReady.Wait();

        var rejected = false;
        var clock = Stopwatch.StartNew();
        while (!rejected && clock.ElapsedMilliseconds < TimeoutMs)
        {
            Arguments[0] = 1;
            try
            {
                method.Invoke(null, Arguments);
            }
            catch (ArgumentException)
            {
                rejected = true;
            }
        }

        stopRacer = true;
        racer.Join();
        return rejected ? 42 : 1;
    }
}
