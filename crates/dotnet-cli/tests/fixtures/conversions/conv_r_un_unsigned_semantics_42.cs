// conv_r_un_unsigned_semantics_42.cs - Ensure conv.r.un treats integer inputs as unsigned
public class Program {
    public static int Main() {
        // int32 -1 interpreted as unsigned should become 2^32 - 1 (exactly representable in double)
        int i32 = -1;
        double u32_as_double = (double)(uint)i32; // ldloc; conv.r.un
        if (u32_as_double != 4294967295.0) return 1;

        // int64 MinValue interpreted as unsigned should become 2^63 (exactly representable)
        long i64_min = long.MinValue;
        double u64_min_as_double = (double)(ulong)i64_min; // ldloc; conv.r.un
        if (u64_min_as_double != 9223372036854775808.0) return 2;

        // int64 -1 interpreted as unsigned is 2^64 - 1; converting to double rounds to 2^64
        long i64 = -1;
        double u64_max_as_double = (double)(ulong)i64;
        if (u64_max_as_double != 18446744073709551616.0) return 3;

        return 42;
    }
}
