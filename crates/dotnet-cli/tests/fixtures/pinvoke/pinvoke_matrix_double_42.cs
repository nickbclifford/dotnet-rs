using System;
using System.Runtime.InteropServices;

public class Program {
    [DllImport("libm", EntryPoint = "fabs")]
    public static extern double fabs(double n);

    public static int Main() {
        double result = fabs(-42.0);
        if (result == 42.0) {
            return 42;
        }
        return (int)result;
    }
}
