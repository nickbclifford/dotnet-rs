using System;

public class Program {
    private const int Iterations = 20_000;

    public static int Main() {
        object target = new object();
        WeakReference reference = new WeakReference(target);
        long checksum = 0;

        for (int i = 0; i < Iterations; i++) {
            object? got = reference.Target;
            if (!ReferenceEquals(got, target)) return 1;
            checksum += i & 0x7;
        }

        GC.KeepAlive(target);
        return checksum > 0 ? 0 : 2;
    }
}
