using System;
using System.Runtime.CompilerServices;
using System.Threading;

public class Subject
{
    public static int Value;
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
    static void Owner() { int value = Subject.Value; if (value == 41) Interlocked.Increment(ref Total); }
    static void Reader()
    {
        Interlocked.Increment(ref Attempted);
        if (Subject.Value == 41) Interlocked.Increment(ref Total);
    }
    static void Legacy()
    {
        Interlocked.Increment(ref Attempted);
        RuntimeHelpers.RunClassConstructor(typeof(Subject).TypeHandle);
        if (Subject.Value == 41) Interlocked.Increment(ref Total);
    }
    public static int Main()
    {
        var owner = new Thread(Owner);
        owner.Start();
        while (Volatile.Read(ref Entered) == 0) Thread.Sleep(1);
        var reader = new Thread(Reader);
        var legacy = new Thread(Legacy);
        reader.Start(); legacy.Start();
        while (Volatile.Read(ref Attempted) != 2) Thread.Sleep(1);
        Thread.Sleep(100);
        Volatile.Write(ref Release, 1);
        owner.Join(); reader.Join(); legacy.Join();
        return Total == 3 ? 42 : 1;
    }
}
