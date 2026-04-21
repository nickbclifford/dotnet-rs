using System;

public class Program {
    private static int Mix(int a, int b, int c, int d) {
        int x = (a + (b << 1)) ^ (c - d);
        int y = ((a >> 3) + (b * 5)) - (c << 1);
        int z = (x ^ y) + (a & 0x7FFF);

        if ((z & 1) == 0) {
            z = (z << 1) ^ (y >> 1);
        } else {
            z = (z >> 1) ^ (x << 1);
        }

        return z & 0x7FFF_FFFF;
    }

    private static int RunRound(int seed, int iterations) {
        int acc = seed;
        int lane0 = seed + 1;
        int lane1 = seed + 3;
        int lane2 = seed + 5;
        int lane3 = seed + 7;

        for (int i = 0; i < iterations; i++) {
            lane0 = Mix(lane0, acc, i, lane3);
            lane1 = Mix(lane1, lane0, i + 1, lane2);
            lane2 = Mix(lane2, lane1, i + 2, lane0);
            lane3 = Mix(lane3, lane2, i + 3, lane1);

            acc ^= lane0 + lane1 - lane2 + lane3;
            acc = (acc << 3) ^ (acc >> 5);
            acc &= 0x7FFF_FFFF;
        }

        return acc ^ lane0 ^ lane1 ^ lane2 ^ lane3;
    }

    public static int Main() {
        int checksum = 17;

        for (int i = 0; i < 4_000; i++) {
            checksum ^= RunRound(i + 11, 10);
            checksum = (checksum << 1) | (checksum >> 30);
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 1;
    }
}
