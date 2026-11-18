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
    private static int targetCalls;
    private static int invalidTargetValues;
    private static readonly ManualResetEventSlim racerReady = new(false);
    private const long TimeoutMs = 30_000;

    private static void Target(ref int value)
    {
        Interlocked.Increment(ref targetCalls);
        if (value != 1)
            Interlocked.Increment(ref invalidTargetValues);
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

        // Direct controls establish the reflection contract independently of scheduling:
        // a wrong boxed type is rejected before the byref target is entered, and the valid
        // control reaches it with the expected value.
        Arguments[0] = 1L;
        var directRejected = false;
        try
        {
            method.Invoke(null, Arguments);
        }
        catch (ArgumentException)
        {
            directRejected = true;
        }
        if (!directRejected || targetCalls != 0) return 1;
        Arguments[0] = 1;
        method.Invoke(null, Arguments);
        if (targetCalls != 1 || invalidTargetValues != 0) return 2;

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
        return rejected && churn > 0 && invalidTargetValues == 0 ? 42 : 3;
    }
}
