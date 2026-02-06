using System;

public class Program {
    public static int Main() {
        if (!TestNaN()) return 1;
        if (!TestInf()) return 2;
        if (!TestNegZero()) return 3;
        if (!TestDivByZero()) return 4;
        
        return 42;
    }

    static bool TestNaN() {
        double nan = double.NaN;
        if (!double.IsNaN(nan)) return false;
        if (nan == nan) return false; // NaN != NaN
        if (nan == 0.0) return false;
        return true;
    }

    static bool TestInf() {
        double inf = double.PositiveInfinity;
        double ninf = double.NegativeInfinity;
        if (!double.IsInfinity(inf)) return false;
        if (!double.IsInfinity(ninf)) return false;
        if (inf + 1.0 != inf) return false;
        if (ninf - 1.0 != ninf) return false;
        if (inf + ninf == inf + ninf) return false; // Inf + (-Inf) = NaN
        return true;
    }

    static bool TestNegZero() {
        double posZero = 0.0;
        double negZero = -0.0;
        if (posZero != negZero) return false; // they should compare equal
        if (1.0 / posZero != double.PositiveInfinity) return false;
        if (1.0 / negZero != double.NegativeInfinity) return false;
        return true;
    }

    static bool TestDivByZero() {
        // Integer division by zero should throw DivideByZeroException
        try {
            int a = 1;
            int b = 0;
            int c = a / b;
            return false;
        } catch (DivideByZeroException) {
            // OK
        }

        // Float division by zero should NOT throw, but return Infinity
        double fa = 1.0;
        double fb = 0.0;
        double fc = fa / fb;
        if (!double.IsInfinity(fc)) return false;

        return true;
    }
}
