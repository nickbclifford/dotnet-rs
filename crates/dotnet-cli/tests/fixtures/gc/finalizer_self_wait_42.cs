using System;
using System.Threading;
using System.Runtime.CompilerServices;

public class Program {
    public static int count;
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Make(int n) { for (int i = 0; i < n; i++) new FWait(); }
    public static int Main() {
        Make(3);
        for (int i = 0; i < 3; i++) {
            GC.Collect();
            GC.WaitForPendingFinalizers();
        }
        Console.WriteLine("done " + count);
        return 42;
    }
}

class FWait { ~FWait() { GC.WaitForPendingFinalizers(); Interlocked.Increment(ref Program.count); } }

