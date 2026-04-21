using System;

public class Program {
    private const int ByteLength = 96 * 1024;
    private const int CharLength = 48 * 1024;
    private const int Iterations = 3_000;

    public static int Main() {
        byte[] leftBytes = new byte[ByteLength + 64];
        byte[] rightBytes = new byte[ByteLength + 64];

        for (int i = 0; i < ByteLength; i++) {
            byte value = (byte)((i * 29 + 5) & 0xFF);
            leftBytes[i] = value;
            rightBytes[i] = value;
        }

        char[] leftChars = new char[CharLength + 64];
        char[] rightChars = new char[CharLength + 64];

        for (int i = 0; i < CharLength; i++) {
            char value = (char)('a' + (i % 26));
            leftChars[i] = value;
            rightChars[i] = value;
        }

        int checksum = 0;

        for (int i = 0; i < Iterations; i++) {
            int byteOffset = (i * 17) & 31;
            int byteLength = ByteLength - byteOffset - 32;
            int byteMismatch = byteOffset + ((i * 131 + 7) % byteLength);

            ReadOnlySpan<byte> leftByteSpan = leftBytes.AsSpan(byteOffset, byteLength);
            ReadOnlySpan<byte> rightByteSpan = rightBytes.AsSpan(byteOffset, byteLength);

            if (!leftByteSpan.SequenceEqual(rightByteSpan)) {
                return 1;
            }

            rightBytes[byteMismatch] ^= 0x3C;
            if (leftByteSpan.SequenceEqual(rightByteSpan)) {
                return 2;
            }
            rightBytes[byteMismatch] ^= 0x3C;

            int charOffset = (i * 13) & 15;
            int charLength = CharLength - charOffset - 16;
            int charMismatch = charOffset + ((i * 97 + 3) % charLength);

            ReadOnlySpan<char> leftCharSpan = leftChars.AsSpan(charOffset, charLength);
            ReadOnlySpan<char> rightCharSpan = rightChars.AsSpan(charOffset, charLength);

            if (!leftCharSpan.SequenceEqual(rightCharSpan)) {
                return 3;
            }

            rightChars[charMismatch] = (char)(rightChars[charMismatch] ^ 0x0020);
            if (leftCharSpan.SequenceEqual(rightCharSpan)) {
                return 4;
            }
            rightChars[charMismatch] = leftChars[charMismatch];

            checksum ^= leftBytes[byteMismatch];
            checksum += leftChars[charMismatch];
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 5;
    }
}
