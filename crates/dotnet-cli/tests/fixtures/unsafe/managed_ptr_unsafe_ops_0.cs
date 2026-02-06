using System;
using System.Runtime.CompilerServices;

public ref struct RefWrapper {
    public ref int Field;
}

public class Program {
    static int global_x = 42;
    static int global_y = 100;
    public static int Main() {
        // Get address of the field containing the ref
        RefWrapper w = new RefWrapper();
        w.Field = ref global_x;
        
        ref byte ptr = ref Unsafe.As<RefWrapper, byte>(ref w);
        
        // Read the RefWrapper (which contains a managed pointer) using Unsafe.ReadUnaligned
        RefWrapper w2 = Unsafe.ReadUnaligned<RefWrapper>(ref ptr);
        
        if (w2.Field != 42) {
            return 1;
        }
        
        // Try writing it back
        RefWrapper wy = new RefWrapper();
        wy.Field = ref global_y;
        Unsafe.WriteUnaligned<RefWrapper>(ref ptr, wy);
        
        if (w.Field != 100) {
            return 2;
        }

        return 0;
    }
}
