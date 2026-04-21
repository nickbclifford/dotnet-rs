using System;

public class Program {
    private const int Length = 64 * 1024;
    private const int Iterations = 2_000;

    public static int Main() {
        char[] baseChars = new char[Length];
        for (int i = 0; i < baseChars.Length; i++) {
            baseChars[i] = (char)('a' + (i % 26));
        }

        char[] mismatchChars = (char[])baseChars.Clone();
        mismatchChars[Length / 3] = 'Z';

        char[] asciiWhitespaceChars = new char[Length];
        for (int i = 0; i < asciiWhitespaceChars.Length; i++) {
            asciiWhitespaceChars[i] = (i & 1) == 0 ? ' ' : '\t';
        }

        char[] asciiMixedChars = (char[])asciiWhitespaceChars.Clone();
        asciiMixedChars[Length / 2] = 'X';

        char[] unicodeWhitespaceChars = new char[Length];
        for (int i = 0; i < unicodeWhitespaceChars.Length; i++) {
            unicodeWhitespaceChars[i] = '\u2003';
        }

        string baseline = new string(baseChars);
        string equal = new string(baseChars);
        string mismatch = new string(mismatchChars);
        string asciiWhitespace = new string(asciiWhitespaceChars);
        string asciiMixed = new string(asciiMixedChars);
        string unicodeWhitespace = new string(unicodeWhitespaceChars);

        int checksum = 0;

        for (int i = 0; i < Iterations; i++) {
            if (!string.Equals(baseline, equal)) {
                return 1;
            }
            if (string.Equals(baseline, mismatch)) {
                return 2;
            }

            int start = (i * 97) & (Length - 1);
            int idx = baseline.IndexOf('m', start);
            if (idx != -1 && idx < start) {
                return 3;
            }

            int missing = baseline.IndexOf('\u2603');
            if (missing != -1) {
                return 4;
            }

            if (!string.IsNullOrWhiteSpace(asciiWhitespace)) {
                return 5;
            }
            if (string.IsNullOrWhiteSpace(asciiMixed)) {
                return 6;
            }
            if (!string.IsNullOrWhiteSpace(unicodeWhitespace)) {
                return 7;
            }

            checksum ^= idx;
            checksum += start;
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 8;
    }
}
