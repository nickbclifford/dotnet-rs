using System;
using System.Runtime.CompilerServices;

public class Program {
    private const int Length = 192 * 1024;
    private const int Iterations = 2_500;

    public static unsafe int Main() {
        byte[] source = new byte[Length + 128];
        byte[] destination = new byte[Length + 128];

        for (int i = 0; i < source.Length; i++) {
            source[i] = (byte)((i * 37 + 9) & 0xFF);
            destination[i] = (byte)((i * 11 + 3) & 0xFF);
        }

        int checksum = 0;

        fixed (byte* sourceBase = source)
        fixed (byte* destinationBase = destination) {
            for (int i = 0; i < Iterations; i++) {
                int srcSkew = (i * 17) & 31;
                int dstSkew = (i * 29) & 31;
                uint copyLength = (uint)(Length - 64);
                byte fill = (byte)((i * 19 + 5) & 0xFF);

                byte* src = sourceBase + srcSkew;
                byte* dst = destinationBase + dstSkew;

                // Non-overlapping copy.
                Unsafe.CopyBlock(dst, src, copyLength);

                // Targeted fill.
                Unsafe.InitBlock(dst + ((i * 7) & 63), fill, 256);

                // Overlapping copy to stress memmove-compatible paths.
                Unsafe.CopyBlock(dst + 5, dst, copyLength - 5);

                int probeA = (i * 193 + 1) % (Length - 64);
                int probeB = (i * 89 + 3) % (Length - 64);
                checksum ^= dst[probeA];
                checksum += dst[probeB];
                checksum &= 0x7FFF_FFFF;
            }
        }

        return checksum != 0 ? 0 : 1;
    }
}
