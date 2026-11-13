using System.Runtime.CompilerServices;

public static class Program
{
    private const int Count = 100;

    public static int Main()
    {
        var table = new ConditionalWeakTable<object, object>();
        var keys = new object[Count];
        for (int i = 0; i < Count; i++)
        {
            keys[i] = new object();
            table.Add(keys[i], i);
        }

        for (int i = 0; i < Count; i++)
        {
            if (!table.TryGetValue(keys[i], out var v) || (int)v != i) return 1;
        }

        if (!table.Remove(keys[50])) return 2;
        if (table.TryGetValue(keys[50], out _)) return 3;
        if (table.Remove(keys[50])) return 4;
        if (!table.TryGetValue(keys[99], out var last) || (int)last != 99) return 5;

        // Re-adding after growth must still work.
        table.Add(keys[50], 5000);
        if (!table.TryGetValue(keys[50], out var again) || (int)again != 5000) return 6;

        System.GC.KeepAlive(keys);
        return 42;
    }
}
