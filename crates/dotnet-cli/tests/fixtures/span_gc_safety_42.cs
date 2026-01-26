using System;
using System.Runtime.CompilerServices;

class Program {
    static int Main() {
        if (!TestSpanGCSafety()) return 101;
        if (!TestInteriorPointer()) return 102;
        return 42;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool TestSpanGCSafety() {
        byte[] array = new byte[100];
        array[0] = 123;
        
        Span<byte> span = array.AsSpan();

        array = null;
        
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        if (span.Length != 100) return false;
        
        if (span[0] != 123) return false;

        span[0] = 200;
        if (span[0] != 200) return false;

        return true;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool TestInteriorPointer() {
        int[] array = new int[10];
        array[5] = 999;

        // array.AsSpan(start, length)
        Span<int> span = array.AsSpan(5, 2);

        array = null;
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        if (span.Length != 2) return false;
        if (span[0] != 999) return false;

        span[0] = 888;
        if (span[0] != 888) return false;

        return true;
    }
}
