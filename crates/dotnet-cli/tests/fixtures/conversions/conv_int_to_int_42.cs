// conv_int_to_int_42.cs - Integer type conversions
public class Program {
    public static int Main() {
        // Test widening conversions
        byte b = 255;
        int i = b;
        if (i != 255) return 1;
        
        // Test narrowing conversions  
        int large = 300;
        byte small = (byte)large;
        if (small != 44) return 2; // 300 % 256
        
        // Test sign extension
        sbyte sb = -1;
        long l = sb;
        if (l != -1) return 3;
        
        // Test zero extension
        byte ub = 255;
        uint ui = ub;
        if (ui != 255) return 4;
        
        return 42;
    }
}
