using System;
using System.Runtime.InteropServices;

public class Program {
    [DllImport("libc", EntryPoint = "llabs")]
    public static extern long llabs(long n);

    public static int Main() {
        long result = llabs(-42L);
        if (result == 42L) {
            return 42;
        }
        return (int)result;
    }
}
