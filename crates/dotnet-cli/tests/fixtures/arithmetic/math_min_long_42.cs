using System;

public class Program {
    public static int Main() {
        long x = 100;
        long y = 4;
        long m = Math.Min(x, y);
        if (m != 4) return 1;
        byte[] buf = new byte[4];
        long n = Math.Min(x - 60, buf.Length);
        if (n != 4) return 2;
        return 42;
    }
}
