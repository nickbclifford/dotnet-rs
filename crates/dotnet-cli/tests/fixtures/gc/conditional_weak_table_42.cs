using System;
using System.Runtime.CompilerServices;

class Program
{
    static ConditionalWeakTable<object, object> table = new ConditionalWeakTable<object, object>();
    static WeakReference keyRef, valueRef, tailRef;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static object Create()
    {
        object key = new object();
        object value = new object();
        object tail = new object();
        tailRef = new WeakReference(tail);
        table.Add(value, tail);
        keyRef = new WeakReference(key);
        valueRef = new WeakReference(value);
        table.Add(key, value);
        return key;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool CheckLive()
    {
        object key = Create();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        bool ok = keyRef.IsAlive && valueRef.IsAlive && tailRef.IsAlive && table.TryGetValue(key, out object value) && value != null;
        GC.KeepAlive(key);
        return ok;
    }

    static int Main()
    {
        if (!CheckLive()) return 1;
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        if (keyRef.IsAlive || valueRef.IsAlive || tailRef.IsAlive) return 2;
        GC.KeepAlive(table);
        return 42;
    }
}
