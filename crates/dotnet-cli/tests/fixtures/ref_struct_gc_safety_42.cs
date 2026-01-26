using System;
using System.Runtime.CompilerServices;

ref struct MySpan
{
    public ref int _reference;
}

public class Program
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    static MySpan CreateSpan(int[] arr)
    {
        MySpan s = new MySpan();
        s._reference = ref arr[0]; 
        return s; 
    }
    
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void TriggerGC()
    {
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
    }

    public static int Main()
    {
        int[] arr = new int[1];
        arr[0] = 42;
        
        // This causes 's' to hold an interior pointer to 'arr'.
        MySpan s = CreateSpan(arr);
        
        // Kill original reference.
        arr = null;
        
        // Trigger GC. If side-table is working, 'arr' should be kept alive by 's._reference'.
        TriggerGC();
        
        // If 'arr' was collected, this access might read garbage or crash.
        // If it's 42, then it's alive.
        if (s._reference == 42)
            return 42;
            
        return 0;
    }
}
