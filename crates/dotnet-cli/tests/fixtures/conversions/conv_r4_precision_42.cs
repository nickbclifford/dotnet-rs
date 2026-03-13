// conv_r4_precision_42.cs - Ensure conv.r4 narrows precision to float32 before re-widening to stack F
public class Program {
    public static int Main() {
        // Precision boundary near 1.0:
        // float32 epsilon at 1.0 is 2^-23, so 1.0 + 2^-25 is not representable as float32 and must round to 1.0.
        double d = 1.0 + (1.0 / 33554432.0); // 2^-25
        double narrowed = (double)(float)d; // ldloc; conv.r4; conv.r8
        if (narrowed != 1.0) return 1;
        if (d == narrowed) return 2;

        // Integer-ish boundary where float32 cannot represent odd integers above 2^24.
        double d2 = 16777217.0; // 2^24 + 1
        double narrowed2 = (double)(float)d2;
        if (narrowed2 != 16777216.0) return 3;

        return 42;
    }
}
