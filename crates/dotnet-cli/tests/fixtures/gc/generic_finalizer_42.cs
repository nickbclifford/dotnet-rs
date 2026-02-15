using System;

public class GenericFinalizable<T> {
    public static int FinalizedCount = 0;
    
    ~GenericFinalizable() {
        FinalizedCount++;
    }
}

public class Program {
    public static void CreateAndDrop() {
        var f = new GenericFinalizable<int>();
    }

    public static int Main() {
        CreateAndDrop();
        
        for (int i = 0; i < 10; i++) {
            GC.Collect();
            GC.WaitForPendingFinalizers();
            if (GenericFinalizable<int>.FinalizedCount > 0) {
                return 42;
            }
        }
        
        return 0;
    }
}
