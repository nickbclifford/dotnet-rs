using System;

public class Program {
    public static int Main() {
        // Infinity
        if (!double.IsPositiveInfinity(1.0 / 0.0)) return 1;
        if (!double.IsNegativeInfinity(-1.0 / 0.0)) return 2;

        // NaN propagation
        double nan = double.NaN;
        if (!double.IsNaN(nan + 1.0)) return 3;
        if (!double.IsNaN(nan * 0.0)) return 4;

        // Zero comparisons
        double posZero = 0.0;
        double negZero = -0.0;
        if (posZero != negZero) return 5; // +0.0 == -0.0
        if (1.0 / posZero != double.PositiveInfinity) return 6;
        if (1.0 / negZero != double.NegativeInfinity) return 7;

        // conv.ovf.i4
        try {
            double d = (double)int.MaxValue + 1000.0;
            int i = checked((int)d);
            return 8;
        } catch (OverflowException) {
            // Expected
        }

        return 42;
    }
}