using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

public class Program {
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static GCHandle CreateWeak() {
        return GCHandle.Alloc(new object(), GCHandleType.Weak);
    }

    public static int Main() {
        GCHandle h = CreateWeak();
        
        GC.Collect();
        GC.WaitForPendingFinalizers();
        
        // After GC, it should be dead
        if (h.Target != null) {
            return 1; // Object should have been collected
        }
        
        h.Free();
        return 42;
    }
}
