using System;
using System.Runtime.CompilerServices;
using System.Threading;

class Program
{
    static ConditionalWeakTable<object, object> table = new ConditionalWeakTable<object, object>();
    static object key;
    static WeakReference keyRef, valueRef;
    static volatile bool ready, release;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void CreateEntry()
    {
        object value = new object();
        valueRef = new WeakReference(value);
        table.Add(key, value);
    }

    static void Creator()
    {
        CreateEntry();
        ready = true;
        while (!release) Thread.Yield();
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool CheckEntry()
    {
        GC.Collect();
        return table.TryGetValue(key, out object value) && value != null && valueRef.Target == value;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static int Exercise()
    {
        key = new object();
        keyRef = new WeakReference(key);
        Thread creator = new Thread(Creator);
        creator.Start();
        try {
            while (!ready) Thread.Yield();
            if (!CheckEntry()) return 1;
            release = true;
            if (!creator.Join(5000)) return 2;
            if (!CheckEntry()) return 3;
            key = null;
            return 42;
        } finally {
            release = true;
            creator.Join();
        }
    }

    static int Main()
    {
        int result = Exercise();
        if (result != 42) return result;
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        if (keyRef.IsAlive || valueRef.IsAlive) return 4;
        GC.KeepAlive(table);
        return 42;
    }
}
