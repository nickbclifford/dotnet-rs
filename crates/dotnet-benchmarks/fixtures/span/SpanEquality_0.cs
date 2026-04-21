using System;

public class Program {
    private const int ByteLength = 128 * 1024;
    private const int CharLength = 32 * 1024;

    public static int Main() {
        byte[] leftBytes = new byte[ByteLength];
        byte[] rightBytes = new byte[ByteLength];
        for (int i = 0; i < ByteLength; i++) {
            byte value = (byte)((i * 37 + 11) & 0xFF);
            leftBytes[i] = value;
            rightBytes[i] = value;
        }

        char[] leftChars = new char[CharLength];
        char[] rightChars = new char[CharLength];
        for (int i = 0; i < CharLength; i++) {
            char value = (char)('a' + (i % 26));
            leftChars[i] = value;
            rightChars[i] = value;
        }

        int checksum = 0;

        ReadOnlySpan<byte> leftByteSpan = leftBytes;
        ReadOnlySpan<byte> rightByteSpan = rightBytes;
        ReadOnlySpan<char> leftCharSpan = leftChars;
        ReadOnlySpan<char> rightCharSpan = rightChars;

        for (int i = 0; i < 2_000; i++) {
            if (!leftByteSpan.SequenceEqual(rightByteSpan)) {
                return 1;
            }

            int byteIndex = (i * 193) % ByteLength;
            rightBytes[byteIndex] ^= 0x5A;
            if (leftByteSpan.SequenceEqual(rightByteSpan)) {
                return 2;
            }
            rightBytes[byteIndex] ^= 0x5A;

            if (!leftCharSpan.SequenceEqual(rightCharSpan)) {
                return 3;
            }

            int charIndex = (i * 131) % CharLength;
            rightChars[charIndex] = (char)(rightChars[charIndex] ^ 0x0020);
            if (leftCharSpan.SequenceEqual(rightCharSpan)) {
                return 4;
            }
            rightChars[charIndex] = leftChars[charIndex];

            checksum ^= leftBytes[byteIndex];
            checksum += leftChars[charIndex];
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 5;
    }
}
