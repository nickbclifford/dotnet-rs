using System;
using System.Runtime.CompilerServices;

public class Program {
    private const int Length = 256 * 1024;
    private const int HalfLength = Length / 2;
    private const int Iterations = 2_000;

    public static unsafe int Main() {
        byte[] source = new byte[Length + 64];
        byte[] destination = new byte[Length + 64];

        for (int i = 0; i < source.Length; i++) {
            source[i] = (byte)((i * 29 + 17) & 0xFF);
            destination[i] = (byte)((i * 7 + 3) & 0xFF);
        }

        int checksum = 0;

        fixed (byte* sourceBase = source)
        fixed (byte* destinationBase = destination) {
            byte* src = sourceBase + 32;
            byte* dst = destinationBase + 16;

            for (int i = 0; i < Iterations; i++) {
                // Non-overlapping copy.
                Unsafe.CopyBlock(dst, src, (uint)Length);

                // Clear then refill half to exercise init/fill throughput.
                Unsafe.InitBlock(dst, 0, (uint)Length);
                byte fill = (byte)((i * 13 + 9) & 0xFF);
                Unsafe.InitBlock(dst + HalfLength, fill, (uint)HalfLength);

                // Overlapping copy to stress memmove-compatible behavior.
                Unsafe.CopyBlock(dst + 1, dst, (uint)(Length - 1));

                checksum ^= dst[(i * 97) & (Length - 1)];
                checksum += dst[(i * 53) & (HalfLength - 1)];
                checksum &= 0x7FFF_FFFF;
            }
        }

        return checksum != 0 ? 0 : 1;
    }
}
