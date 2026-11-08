using System;

public delegate void RefOperation(ref int value);
public static class Later {
    static Later() { Console.WriteLine("cctor"); GC.Collect(); }
    public static void Second(ref int value) { Console.WriteLine("second"); value += 32; }
}
public class Program {
    static void First(ref int value) { Console.WriteLine("first"); value = 10; }
    public static int Main() {
        RefOperation operations = First;
        operations += Later.Second;
        int[] values = new int[1];
        operations(ref values[0]);
        Console.WriteLine(values[0]);
        return values[0] == 42 ? 42 : 1;
    }
}
