using System;

public delegate void RefOperation(ref int value);
public static class LaterRef {
    static LaterRef() { Console.WriteLine("cctor"); }
    public static void Second(ref int target) { Console.WriteLine("second"); target += 32; }
}
public class Program {
    static int value;
    static void First(ref int target) { Console.WriteLine("first"); target = 10; }
    public static int Main() {
        RefOperation operations = First;
        operations += LaterRef.Second;
        operations(ref value);
        Console.WriteLine(value);
        return value == 42 ? 42 : 1;
    }
}
