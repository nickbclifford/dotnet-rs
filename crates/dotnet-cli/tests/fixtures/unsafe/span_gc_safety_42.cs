using System;
using System.Runtime.CompilerServices;

class Program {
    static int Main() {
        int res = TestSpanGCSafety();
        if (res != 42) return res;
        res = TestInteriorPointer();
        if (res != 42) return res;
        return 42;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static int TestSpanGCSafety() {
        byte[] array = new byte[100];
        array[0] = 123;
        
        Span<byte> span = array.AsSpan();

        array = null;
        
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        if (span.Length != 100) return 110;
        
        if (span[0] != 123) return 111;

        span[0] = 200;
        if (span[0] != 200) {
            if (span[0] == 123) return 113;
            return 112;
        }

        return 42;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static int TestInteriorPointer() {
        int[] array = new int[10];
        array[5] = 999;

        // array.AsSpan(start, length)
        Span<int> span = array.AsSpan(5, 2);

        array = null;
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        if (span.Length != 2) return 120;
        if (span[0] != 999) return 121;

        span[0] = 888;
        if (span[0] != 888) return 122;

        return 42;
    }
}
