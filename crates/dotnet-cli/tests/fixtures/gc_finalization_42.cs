using System;

public class Finalizable {
    public static int FinalizedCount = 0;
    
    ~Finalizable() {
        FinalizedCount++;
    }
}

public class Program {
    public static void CreateAndDrop() {
        var f = new Finalizable();
    }

    public static int Main() {
        CreateAndDrop();
        
        GC.Collect();
        GC.WaitForPendingFinalizers();
        
        if (Finalizable.FinalizedCount > 0) {
            return 42;
        }
        return 0;
    }
}
