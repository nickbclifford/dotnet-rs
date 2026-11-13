using System;

public static class Program
{
    static int Expect<TException>(Func<string> f, int code) where TException : Exception
    {
        try { f(); }
        catch (TException) { return 0; }
        return code;
    }

    public static int Main()
    {
        char[] chars = "hello world".ToCharArray();
        if (new string(chars, 6, 5) != "world") return 1;
        if (new string(chars, 0, 0) != "") return 2;
        if (new string(chars, 0, chars.Length) != "hello world") return 4;
        if (new string(chars, 11, 0) != "") return 5;

        char[] surrogates = { 'a', '\uD83D', '\uDE00', 'b' };
        if (new string(surrogates, 1, 2) != "😀") return 6;

        int r;
        if ((r = Expect<ArgumentNullException>(() => new string((char[])null, 0, 0), 10)) != 0) return r;
        if ((r = Expect<ArgumentOutOfRangeException>(() => new string(chars, -1, 1), 11)) != 0) return r;
        if ((r = Expect<ArgumentOutOfRangeException>(() => new string(chars, 0, -1), 12)) != 0) return r;
        if ((r = Expect<ArgumentOutOfRangeException>(() => new string(chars, 7, 5), 13)) != 0) return r;
        if ((r = Expect<ArgumentOutOfRangeException>(() => new string(chars, 12, 0), 14)) != 0) return r;
        if ((r = Expect<ArgumentOutOfRangeException>(() => new string(chars, int.MaxValue, 1), 15)) != 0) return r;

        Console.WriteLine(new string(chars, 0, 5));
        return 42;
    }
}
