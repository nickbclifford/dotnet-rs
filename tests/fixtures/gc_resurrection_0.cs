using System;

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
    public string Name;
    public bool ShouldResurrect = true;

    public Resurrectable(string name) {
        Name = name;
    }

    ~Resurrectable() {
        if (ShouldResurrect) {
            Instance = this;
        }
    }
}

public class Program {
    public static void CreateObjects() {
        var f1 = new Finalizable("f1");
        var f2 = new Finalizable("f2");
        GC.SuppressFinalize(f2);
        var f3 = new Finalizable("f3");
    }

    public static void CreateResurrectable() {
        new Resurrectable("r1");
    }

    public static int Main() {
        CreateObjects();
        GC.Collect();
        GC.WaitForPendingFinalizers();

        CreateResurrectable();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        
        if (Resurrectable.Instance != null) {
            Resurrectable.Instance.ShouldResurrect = false;
            GC.ReRegisterForFinalize(Resurrectable.Instance);
            Resurrectable.Instance = null;
        }

        GC.Collect();
        GC.WaitForPendingFinalizers();

        return 0;
    }
}
