using System;
using System.Threading;

public class Broken
{
    public static int Value;
    public int Instance;
    public Broken() { Instance = 1; }
    public static int Get() { return Value; }
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
    static bool Seen(Exception ex)
    {
        return ex is TypeInitializationException tie && object.ReferenceEquals(tie.InnerException, Inner);
    }
    static void Owner() { try { int v = Broken.Value; } catch (Exception ex) when (Seen(ex)) { Interlocked.Increment(ref Total); } }
    static void ByNew() { Interlocked.Increment(ref Attempted); try { new Broken(); } catch (Exception ex) when (Seen(ex)) { Interlocked.Increment(ref Total); } }
    static void ByCall() { Interlocked.Increment(ref Attempted); try { Broken.Get(); } catch (Exception ex) when (Seen(ex)) { Interlocked.Increment(ref Total); } }
    static void ByDelegate()
    {
        Interlocked.Increment(ref Attempted);
        Func<int> f = Broken.Get;
        try { f(); } catch (Exception ex) when (Seen(ex)) { Interlocked.Increment(ref Total); }
    }
    public static int Main()
    {
        var owner = new Thread(Owner);
        owner.Start();
        while (Volatile.Read(ref Entered) == 0) Thread.Sleep(1);
        var a = new Thread(ByNew);
        var b = new Thread(ByCall);
        var c = new Thread(ByDelegate);
        a.Start(); b.Start(); c.Start();
        while (Volatile.Read(ref Attempted) != 3) Thread.Sleep(1);
        Thread.Sleep(100);
        Volatile.Write(ref Release, 1);
        owner.Join(); a.Join(); b.Join(); c.Join();
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        try { new Broken(); } catch (Exception ex) when (Seen(ex)) { Interlocked.Increment(ref Total); }
        return Total == 5 ? 42 : 1;
    }
}
