using System;

public static class GenericMath {
    public static T Fold<T>(T[] values, Func<T, T, T> op) {
        T acc = values[0];
        for (int i = 1; i < values.Length; i++) {
            acc = op(acc, values[i]);
        }
        return acc;
    }
}

public class Program {
    public static int Main() {
        int[] ints = new int[64];
        long[] longs = new long[64];

        for (int i = 0; i < 64; i++) {
            ints[i] = i + 1;
            longs[i] = i + 1;
        }

        long checksum = 0;
        for (int i = 0; i < 20_000; i++) {
            int intFold = GenericMath.Fold(ints, (a, b) => (a + b) ^ (b << 1));
            long longFold = GenericMath.Fold(longs, (a, b) => (a * 3 + b) ^ (a >> 2));
            checksum += intFold;
            checksum ^= longFold;
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 1;
    }
}
