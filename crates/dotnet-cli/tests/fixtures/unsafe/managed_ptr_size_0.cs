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

        // With Fat Managed Pointers (ptr + origin + checksum), the size is 3 * IntPtr.Size (24 bytes on 64-bit).
        int size = Unsafe.SizeOf<RefStruct>();
        if (size != 3 * IntPtr.Size) {
            return 1;
        }

        return 0;
    }
}
