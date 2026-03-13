using System;

public class Program {
    static int state = 0;

    public static int Main() {
        try {
            Level1();
        } catch (Exception) {
            return 42;
        }
        return 55;
    }

    static void Level1() {
        try {
            throw new Exception("Original");
        } catch (Exception) when (Filter()) {
            state += 100;
        }
    }

    static bool Filter() {
        state += 1;
        throw new NullReferenceException("Filter");
    }
}
