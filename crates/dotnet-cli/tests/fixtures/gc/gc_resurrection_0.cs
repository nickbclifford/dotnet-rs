using System;
using System.Runtime.CompilerServices;

public class Finalizable {
    public static int FinalizedCount = 0;
    public string Name;
    public Finalizable(string name) {
        Name = name;
    }

    ~Finalizable() {
        FinalizedCount++;
    }
}

public class Resurrectable {
    public static Resurrectable? Instance;
    public static int FinalizationCount = 0;
    public string Name;
    public bool ShouldResurrect = true;

    public Resurrectable(string name) {
        Name = name;
    }

    ~Resurrectable() {
        FinalizationCount++;
        if (ShouldResurrect) {
            Instance = this;
        }
    }
}

public class Program {
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void CreateObjects() {
        var f1 = new Finalizable("f1");
        var f2 = new Finalizable("f2");
        GC.SuppressFinalize(f2);
        var f3 = new Finalizable("f3");
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    public static void CreateResurrectable() {
        new Resurrectable("r1");
    }

    public static int Main() {
        CreateObjects();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        if (Finalizable.FinalizedCount != 2) return 1;

        CreateResurrectable();
        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance == null || Resurrectable.FinalizationCount != 1) return 2;
        
        if (Resurrectable.Instance != null) {
            Resurrectable.Instance.ShouldResurrect = false;
            GC.ReRegisterForFinalize(Resurrectable.Instance);
            Resurrectable.Instance = null;
        }

        GC.Collect();
        GC.WaitForPendingFinalizers();

        if (Resurrectable.Instance != null || Resurrectable.FinalizationCount != 2) return 3;

        return 0;
    }
}
