using System;
using System.Runtime.InteropServices;

public class Program {
    private const int Iterations = 20_000;

    public static int Main() {
        object target = new object();
        object other = new object();
        long checksum = 0;

        for (int i = 0; i < Iterations; i++) {
            GCHandle handle = GCHandle.Alloc(target, GCHandleType.Normal);
            if (!ReferenceEquals(handle.Target, target)) return 1;
            handle.Target = other;
            if (!ReferenceEquals(handle.Target, other)) return 2;
            handle.Free();
            checksum += i & 0x7;
        }

        GC.KeepAlive(target);
        GC.KeepAlive(other);
        return checksum > 0 ? 0 : 3;
    }
}
