using System;

public class Greeter
{
    public override string ToString() => "greeter";
}

public struct Point
{
    public int X;
    public override string ToString() => "P" + X;
}

public class Program
{
    static string Format<T>(T value) => $"<{value}>";

    public static int Main()
    {
        object obj = new Greeter();
        if ($"{obj}" != "greeter") return 1;
        Greeter g = new Greeter();
        if ($"[{g}]" != "[greeter]") return 2;
        Point p = new Point { X = 7 };
        if ($"{p}" != "P7") return 3;
        if (Format(g) != "<greeter>") return 4;
        if (Format(p) != "<P7>") return 5;
        if ($"{5}" != "5") return 6;
        return 42;
    }
}
