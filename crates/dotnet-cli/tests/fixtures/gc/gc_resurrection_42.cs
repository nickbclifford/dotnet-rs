using System;

public class Resurrectable {
    public static Resurrectable Instance;
    public bool ShouldResurrect = true;
    public int ResurrectCount = 0;

    ~Resurrectable() {
        if (ShouldResurrect) {
            ResurrectCount++;
            Instance = this;
        }
    }
}

public class Program {
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

        // 2nd GC - should NOT trigger finalizer because it already ran and wasn't reregistered
        Resurrectable.Instance = null;
        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance != null) {
            return 3; // Should NOT have resurrected
        }

        // Create another one and test reregistration
        var r = new Resurrectable();
        r.ShouldResurrect = true;
        r = null;

        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance == null) {
            return 4; // Failed second resurrection
        }

        // Reregister for second finalization
        GC.ReRegisterForFinalize(Resurrectable.Instance);
        Resurrectable.Instance.ShouldResurrect = false; // Next time, don't resurrect
        Resurrectable.Instance = null;

        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance != null) {
            return 5; // Should not have resurrected this time
        }

        // Everything passed
        return 42;
    }
}
