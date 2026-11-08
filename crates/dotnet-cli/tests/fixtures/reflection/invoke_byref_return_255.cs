using System;

public static class Program
{
    public static ref int Target(ref int value)
    {
        Console.WriteLine("target ran");
        return ref value;
    }

    public static int Main()
    {
        object[] args = { null! };
        typeof(Program).GetMethod(nameof(Target))!.Invoke(null, args);
        return 42;
    }
}
