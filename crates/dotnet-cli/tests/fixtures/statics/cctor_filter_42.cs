using System;

public class Broken
{
    public static int Value;
    static Broken() { throw new InvalidOperationException("initializer inner"); }
}

public class Program
{
    static bool Filter(Exception ex)
    {
        Console.WriteLine(ex.GetType().Name);
        return ex is TypeInitializationException tie &&
            tie.InnerException is InvalidOperationException inner &&
            inner.Message == "initializer inner";
    }

    public static int Main()
    {
        try { return Broken.Value; }
        catch (Exception ex) when (Filter(ex)) { return 42; }
    }
}
