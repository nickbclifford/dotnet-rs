using System;
using System.Reflection;

public class Program {
    static bool Matches(string result, string source, int start, int length) {
        if (result.Length != length) return false;
        for (int i = 0; i < length; i++) {
            if (result[i] != source[start + i]) return false;
        }
        return true;
    }

    public static int Main() {
        char[] chars = new char[65540];
        for (int i = 0; i < chars.Length; i++) chars[i] = (char)('a' + i % 26);
        string source = new string(chars);

        Func<int, string> one = source.Substring;
        Func<int, int, string> two = source.Substring;
        if (!Matches(one(2), source, 2, 65538)) return 1;
        if (!Matches(two(1, 65537), source, 1, 65537)) return 2;

        MethodInfo oneMethod = typeof(string).GetMethod("Substring", new Type[] { typeof(int) });
        MethodInfo twoMethod = typeof(string).GetMethod("Substring", new Type[] { typeof(int), typeof(int) });
        if (oneMethod == null || twoMethod == null) return 3;
        string oneResult = (string)oneMethod.Invoke(source, new object[] { 2 });
        string twoResult = (string)twoMethod.Invoke(source, new object[] { 1, 65537 });
        if (!Matches(oneResult, source, 2, 65538)) return 4;
        if (!Matches(twoResult, source, 1, 65537)) return 5;
        return 42;
    }
}
