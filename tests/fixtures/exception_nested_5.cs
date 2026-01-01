using System;

public class Program {
    static int state = 0;
    public static int Main() {
        try {
            Level1();
        } catch (Exception) {
            state += 4;
        }
        return state;
    }

    static void Level1() {
        try {
            Level2();
        } catch (Exception) when (Filter()) {
            state += 100;
        }
    }

    static bool Filter() {
        state += 1;
        throw new Exception();
    }

    static void Level2() {
        try {
            throw new Exception();
        } finally {
            state += 2;
        }
    }
}
