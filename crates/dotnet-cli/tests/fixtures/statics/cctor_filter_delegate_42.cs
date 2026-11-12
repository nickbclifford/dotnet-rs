using System;
using System.Reflection;

public class Broken
{
    public static int Value;
    public int Instance;
    static Broken() { throw new InvalidOperationException("initializer inner"); }
    public Broken() { Instance = 1; }
    public static int Call() { return 1; }
    public static void Run() { }
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

    static int Trigger()
    {
        Func<int> f = Broken.Call; return f();
    }

    public static int Main()
    {
        try { return Trigger(); }
        catch (Exception ex) when (Filter(ex)) { return 42; }
    }
}
