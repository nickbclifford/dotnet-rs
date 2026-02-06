using System;

public class Program {
    static int result = 0;
    public static int Main() {
        try {
            TrySomething();
        } catch (Exception) {
            return result;
        }
        return 0;
    }

    static void TrySomething() {
        try {
            throw new Exception();
        } finally {
            result = 42;
        }
    }
}
