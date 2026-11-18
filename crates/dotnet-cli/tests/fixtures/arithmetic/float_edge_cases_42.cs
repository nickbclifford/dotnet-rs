using System;

public class Program {
    static double One() => 1.0;
    static double NegativeOne() => -1.0;
    static double Zero() => 0.0;

    public static int Main() {
        // Infinity
        double one = One();
        double negativeOne = NegativeOne();
        double zero = Zero();
        if (!double.IsPositiveInfinity(one / zero)) return 1;
        if (!double.IsNegativeInfinity(negativeOne / zero)) return 2;

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
