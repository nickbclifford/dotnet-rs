using System;

public sealed class FinalizerAfterEntrypoint
{
    ~FinalizerAfterEntrypoint()
    {
        Console.WriteLine("finalizer ran");
    }
}

public static class Program
{
    public static void Main()
    {
        new FinalizerAfterEntrypoint();
        GC.Collect();
    }
}
