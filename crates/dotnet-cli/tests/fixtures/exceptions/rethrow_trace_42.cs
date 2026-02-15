using System;

public class Program {
    public static int Main() {
        try {
            A();
        } catch (Exception e) {
            string st = e.StackTrace;
            if (ManualContains(st, "at Program.B") && 
                ManualContains(st, "at Program.A") && 
                ManualContains(st, "at Program.Main")) {
                return 42;
            }
        }
        return 1;
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

    public static void A() {
        try {
            B();
        } catch {
            throw; // Rethrow
        }
    }

    public static void B() {
        throw new Exception("Test");
    }
}
