using System;
using System.Runtime.CompilerServices;
using System.Threading;

public class Counter
{
    public int Value;
    public bool Overlap;
    int inside;

    [MethodImpl(MethodImplOptions.Synchronized)]
    public void Bump()
    {
        if (++inside != 1) Overlap = true;
        int v = Value;
        Thread.Sleep(0);
        Value = v + 1;
        inside--;
    }

    [MethodImpl(MethodImplOptions.Synchronized)]
    public void Throwing() { throw new InvalidOperationException(); }

    [MethodImpl(MethodImplOptions.Synchronized)]
    public int Reentrant(int n) => n == 0 ? 0 : 1 + Reentrant(n - 1);

    public bool HeldByThis() => Monitor.IsEntered(this);

    [MethodImpl(MethodImplOptions.Synchronized)]
    public bool HeldInside() => Monitor.IsEntered(this);
}

public static class Program
{
    static int sInside;
    static bool sOverlap;
    static int sValue;

    [MethodImpl(MethodImplOptions.Synchronized)]
    static void SBump()
    {
        if (++sInside != 1) sOverlap = true;
        int v = sValue;
        Thread.Sleep(0);
        sValue = v + 1;
        sInside--;
    }

    [MethodImpl(MethodImplOptions.Synchronized)]
    static bool StaticHeldInside() => Monitor.IsEntered(typeof(Program));

    public static int Main()
    {
        var c = new Counter();
        if (c.HeldByThis()) return 1;
        if (!c.HeldInside()) return 2;
        if (c.HeldByThis()) return 3;
        try { c.Throwing(); return 4; } catch (InvalidOperationException) { }
        if (c.HeldByThis()) return 5;
        if (c.Reentrant(3) != 3) return 6;
        if (c.HeldByThis()) return 7;
        if (!StaticHeldInside()) return 8;
        if (Monitor.IsEntered(typeof(Program))) return 9;

        var threads = new Thread[4];
        for (int i = 0; i < threads.Length; i++)
        {
            threads[i] = new Thread(() =>
            {
                for (int k = 0; k < 50; k++) { c.Bump(); SBump(); }
            });
            threads[i].Start();
        }
        foreach (var t in threads) t.Join();
        if (c.Overlap || sOverlap) return 10;
        if (c.Value != 200 || sValue != 200) return 11;
        return 42;
    }
}
