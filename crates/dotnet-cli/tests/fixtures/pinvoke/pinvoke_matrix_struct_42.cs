using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)]
public struct DivT {
    public int Quot;
    public int Rem;
}

public class Program {
    [DllImport("libc", EntryPoint = "div")]
    public static extern DivT div(int numer, int denom);

    public static int Main() {
        DivT result = div(100, 3);
        // 100 / 3 = 33, rem 1
        if (result.Quot == 33 && result.Rem == 1) {
            return 42;
        }
        return result.Quot;
    }
}
