using System;
using System.Threading;

public static class Program
{
    static int ready;

    static void Worker()
    {
        Volatile.Write(ref ready, 1);
        Thread.Sleep(Timeout.Infinite);
    }

    public static void Main()
    {
        new Thread(Worker).Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        Console.WriteLine("before entry failure");
        throw new InvalidOperationException("entry failure");
    }
}
