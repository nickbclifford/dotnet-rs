using System;
using System.Threading;

public static class Program
{
    static int ready;

    static void Worker()
    {
        Volatile.Write(ref ready, 1);
        while (true) Thread.Sleep(50);
    }

    public static void Main()
    {
        new Thread(Worker).Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        Console.WriteLine("before entry failure");
        throw new InvalidOperationException("entry failure");
    }
}
