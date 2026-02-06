// File: pinvoke/libc_42.cs
// Purpose: Test simple P/Invoke call
// Expected: Returns 42 on success

using System;
using System.Runtime.InteropServices;

public class Program {
    // On Linux, libc is usually just "libc" or "libc.so.6"
    // The VM should be able to find it.
    [DllImport("libc")]
    public static extern int abs(int n);

    public static int Main() {
        int result = abs(-42);
        if (result == 42) {
            return 42;
        }
        return result;
    }
}
