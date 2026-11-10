using System;

public static class Program {
    public static int Main() {
        char[] chars = new char[131080];
        for (int i = 0; i < chars.Length; i++) chars[i] = (char)('a' + i % 26);
        string source = new string(chars);
        Func<int, int, string> substring = source.Substring;
        string result = substring(3, 131073);
        if (result.Length != 131073) return 1;
        for (int i = 0; i < result.Length; i++) if (result[i] != chars[i + 3]) return 2;
        return 42;
    }
}
