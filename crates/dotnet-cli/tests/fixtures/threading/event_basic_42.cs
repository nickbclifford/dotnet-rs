using System;
using System.Threading;
using System.Reflection;

static class Program
{
    static int Main()
    {
        try { _ = new EventWaitHandle(false, (EventResetMode)2); return 9; }
        catch (ArgumentException) { }
        using var initiallyManual = new ManualResetEvent(true);
        if (!initiallyManual.WaitOne(0) || !initiallyManual.WaitOne(0)) return 10;
        using var auto = new AutoResetEvent(true);
        if (!auto.WaitOne(0) || auto.WaitOne(0)) return 1;
        if (!auto.Set() || !auto.WaitOne(0) || auto.WaitOne(1)) return 2;
        auto.Set(); auto.Set();
        if (!auto.WaitOne(0) || auto.WaitOne(0)) return 12;
        using var manual = new ManualResetEvent(false);
        if (manual.WaitOne(0) || !manual.Set() || !manual.WaitOne(0) || !manual.WaitOne(0)) return 3;
        if (!manual.Reset() || manual.WaitOne(0)) return 4;
        var setSafe = typeof(EventWaitHandle).GetMethod("Set", BindingFlags.Static | BindingFlags.NonPublic)!;
        if (!(bool)setSafe.Invoke(null, new object[] { manual.SafeWaitHandle })! || !manual.WaitOne(0)) return 7;
        GC.Collect(); GC.WaitForPendingFinalizers();
        if (!manual.WaitOne(0)) return 8;
        manual.Dispose();
        try { manual.WaitOne(0); return 5; } catch (ObjectDisposedException) { }
        try { manual.Set(); return 6; } catch (ObjectDisposedException) { }
        try { manual.Reset(); return 11; } catch (ObjectDisposedException) { }
        return 42;
    }
}
