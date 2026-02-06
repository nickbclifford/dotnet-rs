// conv_float_to_int_42.cs - Float to integer conversions
public class Program {
    public static int Main() {
        double d = 42.9;
        int i = (int)d;
        if (i != 42) return 1; // Truncation
        
        float f = -3.7f;
        int j = (int)f;
        if (j != -3) return 2; // Truncation toward zero
        
        return 42;
    }
}
