using System;
using System.Runtime.CompilerServices;
using System.Threading;

class LockedPayload
{
    ~LockedPayload()
    {
        Program.LockStarted = true;
        lock (Program.Gate) { Program.LockDone = true; }
    }
}

class InitializingType
{
    public static int Value;
    static InitializingType()
    {
        Program.InitStarted = true;
        while (!Program.ReleaseInit) Thread.Sleep(1);
        Value = 42;
    }
}

class InitializerPayload
{
    ~InitializerPayload()
    {
        Program.FinalizerStarted = true;
        Program.Observed = InitializingType.Value;
        Program.FinalizerDone = true;
    }
}

class Program
{
    public static object Gate = new object();
    public static volatile bool LockStarted, LockDone, InitStarted, ReleaseInit;
    public static volatile bool FinalizerStarted, FinalizerDone;
    public static int Observed;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void MakeLocked() { new LockedPayload(); }
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void MakeInitializer() { new InitializerPayload(); }
    static void Initialize() { int value = InitializingType.Value; }

    static bool WaitFor(Func<bool> condition)
    {
        for (int i = 0; i < 5000; i++)
        {
            if (condition()) return true;
            Thread.Sleep(1);
        }
        return false;
    }

    static int Main()
    {
        MakeLocked();
        lock (Gate)
        {
            GC.Collect();
            if (!WaitFor(() => LockStarted) || LockDone) return 1;
        }
        GC.WaitForPendingFinalizers();
        if (!LockDone) return 2;

        var initializer = new Thread(Initialize);
        initializer.Start();
        try
        {
            if (!WaitFor(() => InitStarted)) return 3;
            MakeInitializer();
            GC.Collect();
            if (!WaitFor(() => FinalizerStarted) || FinalizerDone) return 4;
        }
        finally
        {
            ReleaseInit = true;
            initializer.Join();
        }
        GC.WaitForPendingFinalizers();
        return FinalizerDone && Observed == 42 ? 42 : 5;
    }
}
