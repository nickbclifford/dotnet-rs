using System;
using System.Runtime.CompilerServices;

public class Broken
{
    static Broken() { throw new InvalidOperationException("initializer inner"); }
}

public class Subject
{
    public static int Value;
    static Subject()
    {
        RuntimeHelpers.RunClassConstructor(typeof(Subject).TypeHandle);
        Value = ++Program.Count;
    }
}

public class DelegateSubject
{
    static DelegateSubject() { ++Program.Count; }
}

public class Program
{
    public static int Count;
    static bool Filter(Exception ex)
    {
        Console.WriteLine(ex.GetType().Name);
        return ex is TypeInitializationException tie &&
            tie.InnerException is InvalidOperationException inner &&
            inner.Message == "initializer inner";
    }

    public static int Main()
    {
        try { RuntimeHelpers.RunClassConstructor(typeof(Broken).TypeHandle); }
        catch (Exception ex) when (Filter(ex))
        {
            RuntimeHelpers.RunClassConstructor(typeof(Subject).TypeHandle);
            RuntimeHelpers.RunClassConstructor(typeof(Subject).TypeHandle);
            Action<RuntimeTypeHandle> run = RuntimeHelpers.RunClassConstructor;
            run(typeof(DelegateSubject).TypeHandle);
            return Count == 2 && Subject.Value == 1 ? 42 : 1;
        }
        return 1;
    }
}
