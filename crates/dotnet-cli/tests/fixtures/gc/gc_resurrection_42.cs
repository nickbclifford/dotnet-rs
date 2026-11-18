using System;
using System.Runtime.CompilerServices;

public class Resurrectable {
    public static Resurrectable Instance;
    public static int FinalizedCount;
    public bool ShouldResurrect = true;
    public int ResurrectCount = 0;

    ~Resurrectable() {
        FinalizedCount++;
        if (ShouldResurrect) {
            ResurrectCount++;
            Instance = this;
        }
    }
}

public class Program {
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void Create() {
        new Resurrectable();
    }

    public static int Main() {
        Create();
        
        // 1st GC - should trigger finalizer and resurrect
        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance == null) {
            return 1; // Failed to resurrect
        }

        if (Resurrectable.Instance.ResurrectCount != 1) {
            return 2; // Finalizer didn't run or ran too many times
        }
        if (Resurrectable.FinalizedCount != 1) return 6;

        // 2nd GC - should NOT trigger finalizer because it already ran and wasn't reregistered
        Resurrectable.Instance = null;
        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance != null) {
            return 3; // Should NOT have resurrected
        }
        if (Resurrectable.FinalizedCount != 1) return 7;

        // Create another one and test reregistration
        Create();

        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance == null) {
            return 4; // Failed second resurrection
        }
        if (Resurrectable.FinalizedCount != 2) return 8;

        // Reregister for second finalization
        GC.ReRegisterForFinalize(Resurrectable.Instance);
        Resurrectable.Instance.ShouldResurrect = false; // Next time, don't resurrect
        Resurrectable.Instance = null;

        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance != null) {
            return 5; // Should not have resurrected this time
        }
        if (Resurrectable.FinalizedCount != 3) return 9;

        // Everything passed
        return 42;
    }
}
