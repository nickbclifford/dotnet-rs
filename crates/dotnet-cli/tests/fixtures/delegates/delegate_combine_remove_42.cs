using System;

public delegate int MyDelegate(int x);

public class Program {
    public static int Double(int x) => x * 2;
    public static int Triple(int x) => x * 3;

    public static int Main() {
        MyDelegate d1 = Double;
        MyDelegate d2 = Triple;

        // Combine
        MyDelegate c1 = (MyDelegate)Delegate.Combine(d1, d2);
        if (c1 == null) return 1;
        
        // Test Equality of combined delegates
        MyDelegate c2 = (MyDelegate)Delegate.Combine(d1, d2);
        if (!c1.Equals(c2)) return 2;

        // Test invocation (indirectly, as we know multicast works if targets are set)
        int res = c1(10);
        if (res != 30) return 3; // Triple(10) = 30 (last one wins)

        // Remove
        MyDelegate r1 = (MyDelegate)Delegate.Remove(c1, d2);
        if (r1 == null) return 4;
        if (!r1.Equals(d1)) return 5;
        if (r1(10) != 20) return 6;

        MyDelegate r2 = (MyDelegate)Delegate.Remove(c1, d1);
        if (r2 == null) return 7;
        if (!r2.Equals(d2)) return 8;
        if (r2(10) != 30) return 9;

        // Remove everything
        MyDelegate r3 = (MyDelegate)Delegate.Remove(r1, d1);
        if (r3 != null) return 10;

        // Combine null
        if (Delegate.Combine(d1, null) != d1) return 11;
        if (Delegate.Combine(null, d2) != d2) return 12;

        // Remove non-existent
        if (Delegate.Remove(d1, d2) != d1) return 13;

        return 42;
    }
}
