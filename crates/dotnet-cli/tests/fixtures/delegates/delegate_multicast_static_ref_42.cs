using System;

public delegate void RefAction(ref int value);
public class Program {
    static int value;
    static void First(ref int target) {
        target = 10;
        Console.WriteLine(value);
    }
    static void Second(ref int target) {
        value += 1;
        target += 31;
        Console.WriteLine(value);
    }
    public static int Main() {
        RefAction targets = First;
        targets += Second;
        targets(ref value);
        return value == 42 ? 42 : 1;
    }
}
