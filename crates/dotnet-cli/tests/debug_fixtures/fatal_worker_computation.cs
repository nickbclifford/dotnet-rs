using System;
using System.Threading;

public static class Program
{
    static int ready;
    static void Compute()
    {
        Volatile.Write(ref ready, 1);
        long value = 0;
        while (true) { value++; if (value == long.MaxValue) Console.WriteLine("unexpected completion"); }
    }
    static void Fail()
    {
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        throw new InvalidOperationException("worker failure");
    }
    public static int Main()
    {
        Console.WriteLine("before worker");
        new Thread(Compute).Start();
        new Thread(Fail).Start();
        while (true) Thread.Sleep(1);
    }
}
