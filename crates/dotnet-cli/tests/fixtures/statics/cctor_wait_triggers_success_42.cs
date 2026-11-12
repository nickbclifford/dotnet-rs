using System;
using System.Threading;

public class Subject
{
    public static int Value;
    public int Instance;
    public Subject() { Instance = Value; }
    public static int Get() { return Value; }
    static Subject()
    {
        Volatile.Write(ref Program.Entered, 1);
        while (Volatile.Read(ref Program.Release) == 0) Thread.Sleep(1);
        Value = 41;
    }
}

public class Program
{
    public static int Entered, Release, Attempted, Total;
    static Program() { }
    static void Owner() { if (Subject.Value == 41) Interlocked.Increment(ref Total); }
    static void ByNew() { Interlocked.Increment(ref Attempted); if (new Subject().Instance == 41) Interlocked.Increment(ref Total); }
    static void ByCall() { Interlocked.Increment(ref Attempted); if (Subject.Get() == 41) Interlocked.Increment(ref Total); }
    static void ByDelegate()
    {
        Interlocked.Increment(ref Attempted);
        Func<int> f = Subject.Get;
        if (f() == 41) Interlocked.Increment(ref Total);
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
        return Total == 4 ? 42 : 1;
    }
}
