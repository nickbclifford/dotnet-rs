using System;
using System.Runtime.CompilerServices;
using System.Threading;

public class Broken
{
    public static int Value;
    static Broken()
    {
        Volatile.Write(ref Program.Entered, 1);
        while (Volatile.Read(ref Program.Release) == 0) Thread.Sleep(1);
        Program.Inner = new InvalidOperationException("worker initializer");
        throw Program.Inner;
    }
}

public class Program
{
    public static int Entered, Release, Attempted, Total;
    public static Exception Inner;
    static Program() { }
    static void Access()
    {
        try { int value = Broken.Value; }
        catch (TypeInitializationException ex)
        {
            if (object.ReferenceEquals(ex.InnerException, Inner)) Interlocked.Increment(ref Total);
        }
    }
    static void Waiter() { Interlocked.Increment(ref Attempted); Access(); }
    static void Legacy()
    {
        Interlocked.Increment(ref Attempted);
        try { RuntimeHelpers.RunClassConstructor(typeof(Broken).TypeHandle); }
        catch (TypeInitializationException) { Interlocked.Increment(ref Total); }
    }
    public static int Main()
    {
        var owner = new Thread(Access);
        owner.Start();
        while (Volatile.Read(ref Entered) == 0) Thread.Sleep(1);
        var first = new Thread(Waiter);
        var second = new Thread(Waiter);
        var legacy = new Thread(Legacy);
        first.Start(); second.Start(); legacy.Start();
        while (Volatile.Read(ref Attempted) != 3) Thread.Sleep(1);
        Thread.Sleep(100);
        Volatile.Write(ref Release, 1);
        owner.Join(); first.Join(); second.Join(); legacy.Join();
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        Access();
        return Total == 5 ? 42 : 1;
    }
}
