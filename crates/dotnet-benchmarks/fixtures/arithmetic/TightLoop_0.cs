using System;

public class Program {
    public static int Main() {
        long acc = 1;
        for (int i = 1; i <= 200_000; i++) {
            acc += (i * 13L) ^ (i >> 3);
            acc ^= (acc << 7) | (acc >> 11);
            acc &= 0x7FFF_FFFF;
        }

        return acc != 0 ? 0 : 1;
    }
}
