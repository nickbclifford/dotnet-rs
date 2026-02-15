using System;

public class Program {
    public static int Main() {
        A();
        return 42;
    }

    public static void A() {
        GC.Collect(-1);
    }

    public static void A_Inner() {
    }

    public static bool ManualContains(string haystack, string needle) {
        if (haystack == null || needle == null) return false;
        if (needle.Length == 0) return true;
        for (int i = 0; i <= haystack.Length - needle.Length; i++) {
            bool found = true;
            for (int j = 0; j < needle.Length; j++) {
                if (haystack[i + j] != needle[j]) {
                    found = false;
                    break;
                }
            }
            if (found) return true;
        }
        return false;
    }
}
