using System;

public delegate int Operation(int value);
public static class Later {
    static Later() { Console.WriteLine("cctor"); }
    public static int Second(int value) { Console.WriteLine("second"); return value + 20; }
}
public class Program {
    static int First(int value) { Console.WriteLine("first"); return value; }
    public static int Main() {
        Operation operations = First;
        operations += Later.Second;
        int result = operations(10);
        Console.WriteLine(result);
        return result == 30 ? 42 : 1;
    }
}
