using System;

public static class Program
{
    public static int Main()
    {
        var method = typeof(Math).GetMethod(nameof(Math.Abs), new[] { typeof(int) })!;
        object result = method.Invoke(null, new object[] { -42 })!;
        return result is int value && value == 42 ? 42 : 1;
    }
}
