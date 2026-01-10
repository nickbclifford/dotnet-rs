using System;
using System.Runtime.CompilerServices;

public ref struct RefStruct {
    public ref int Field;
}

public class Program {
    static int global_x = 42;
    public static int Main() {
        RefStruct s = new RefStruct();
        s.Field = ref global_x;

        // On 64-bit, IntPtr is 8 bytes.
        // If ManagedPtr is 33+ bytes, this will be much larger.
        int size = Unsafe.SizeOf<RefStruct>();
        if (size != IntPtr.Size) {
            return 1;
        }

        return 0;
    }
}
