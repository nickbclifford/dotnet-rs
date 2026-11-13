using System;
using System.Runtime.CompilerServices;

// Keep Keys <= 8: adding a ninth entry to a ConditionalWeakTable crashes the VM with SIGSEGV
// (reproduced at 03cb031d and at HEAD, single-threaded and multithreaded; see plan 20).
public class Program {
    private const int Keys = 8;
    private const int Rounds = 2500;

    public static int Main() {
        var table = new ConditionalWeakTable<object, object>();
        var keys = new object[Keys];
        var values = new object[Keys];
        for (int i = 0; i < Keys; i++) {
            keys[i] = new object();
            values[i] = new object();
            table.Add(keys[i], values[i]);
        }

        long checksum = 0;
        for (int round = 0; round < Rounds; round++) {
            for (int i = 0; i < Keys; i++) {
                if (!table.TryGetValue(keys[i], out object? found) || !ReferenceEquals(found, values[i])) return 1;
                checksum += i & 0x7;
            }
        }

        GC.KeepAlive(table);
        GC.KeepAlive(keys);
        GC.KeepAlive(values);
        return checksum > 0 ? 0 : 2;
    }
}
