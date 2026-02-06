using System;

public class Program {
    public static int Main() {
        if (!TestLength()) return 1;
        if (!TestIndexing()) return 2;
        if (!TestEquality()) return 3;
        if (!TestConcat()) return 4;
        if (!TestIsNullOrEmpty()) return 5;
        if (!TestUnicode()) return 6;
        
        return 42;
    }

    static bool TestLength() {
        string s = "Hello";
        return s.Length == 5;
    }

    static bool TestIndexing() {
        string s = "Hello";
        return s[0] == 'H' && s[4] == 'o';
    }

    static bool TestEquality() {
        string s1 = "Hello";
        string s2 = "Hello";
        string s3 = "World";
        string s4 = "hello"; // different case
        
        if (!s1.Equals(s2)) return false;
        if (s1 == s3) return false;
        if (s1 == s4) return false;
        return true;
    }

    static bool TestConcat() {
        string s1 = "Hello";
        string s2 = " World";
        // string s3 = s1 + s2; // This might call String.Concat(string, string) which might not be an intrinsic
        // Let's try explicit Concat if + fails or just use it.
        string s3 = string.Concat(s1, s2);
        return s3 == "Hello World";
    }

    static bool TestIsNullOrEmpty() {
        if (!string.IsNullOrEmpty("")) return false;
        if (!string.IsNullOrEmpty(null)) return false;
        if (string.IsNullOrEmpty("a")) return false;
        return true;
    }

    static bool TestUnicode() {
        string s = "こんにちは"; // "Konnichiwa" in Japanese
        if (s.Length != 5) return false;
        if (s[0] != '\u3053') return false; // 'こ'
        return true;
    }
}
