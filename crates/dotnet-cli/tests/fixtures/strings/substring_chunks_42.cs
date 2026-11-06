using System;

public class Program {
    static bool Matches(string actual, string source, int start, int length) {
        if (actual.Length != length) return false;
        for (int i = 0; i < length; i++) {
            if (actual[i] != source[start + i]) return false;
        }
        return true;
    }

    static bool ThrowsRange(Action action) {
        try { action(); }
        catch (ArgumentOutOfRangeException exception) {
            // The differential test checks parameter names against the CLR too.
            Console.WriteLine(exception.ParamName);
            return true;
        }
        return false;
    }

    static bool InFilter(string source) {
        return Matches(source.Substring(1, 65537), source, 1, 65537);
    }

    static bool HandlerContexts(string source) {
        bool caught = false;
        bool finalized = false;
        try {
            try { throw new InvalidOperationException(); }
            catch (InvalidOperationException) when (InFilter(source)) {
                caught = Matches(source.Substring(2, 65537), source, 2, 65537);
            }
        } finally {
            finalized = Matches(source.Substring(3, 65537), source, 3, 65537);
        }
        return caught && finalized;
    }

    public static int Main() {
        char[] chars = new char[196620];
        for (int i = 0; i < chars.Length; i++) {
            // Include a pair and isolated high and low surrogates in every chunk.
            switch (i % 7) {
                case 0: chars[i] = '\uD83D'; break;
                case 1: chars[i] = '\uDE00'; break;
                case 2: chars[i] = '\uD800'; break;
                case 3: chars[i] = 'x'; break;
                case 4: chars[i] = '\uDC00'; break;
                case 5: chars[i] = '\u3053'; break;
                default: chars[i] = (char)('a' + i % 26); break;
            }
        }
        string source = new string(chars);
        int[] lengths = { 0, 1, 65535, 65536, 65537, 131072, 131073, 196617 };
        foreach (int length in lengths) {
            if (!Matches(source.Substring(3, length), source, 3, length)) return 1;
            int start = source.Length - length;
            if (!Matches(source.Substring(start), source, start, length)) return 2;
        }
        if (source.Substring(source.Length) != "") return 3;
        if (source.Substring(source.Length, 0) != "") return 4;

        // Keep both results live across consecutive resumable calls.
        string first = source.Substring(1, 131073);
        string second = source.Substring(2, 131073);
        if (!Matches(first, source, 1, 131073) || !Matches(second, source, 2, 131073)) return 5;
        if (!HandlerContexts(source)) return 6;

        if (!ThrowsRange(() => source.Substring(-1))) return 7;
        if (!ThrowsRange(() => source.Substring(source.Length + 1))) return 8;
        if (!ThrowsRange(() => source.Substring(-1, 0))) return 9;
        if (!ThrowsRange(() => source.Substring(0, -1))) return 10;
        if (!ThrowsRange(() => source.Substring(source.Length, 1))) return 11;
        if (!ThrowsRange(() => source.Substring(int.MaxValue, int.MaxValue))) return 12;
        if (!ThrowsRange(() => source.Substring(1, int.MaxValue))) return 13;
        string missing = null;
        try { missing.Substring(0); return 14; }
        catch (NullReferenceException) { }
        try { missing.Substring(0, 0); return 15; }
        catch (NullReferenceException) { }
        return 42;
    }
}
