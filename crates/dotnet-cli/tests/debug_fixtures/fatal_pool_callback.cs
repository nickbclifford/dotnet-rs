using System;
using System.Threading;

public static class Program
{
    public static int Main()
    {
        Console.WriteLine("before worker");
        ThreadPool.QueueUserWorkItem(_ => throw new InvalidOperationException("worker failure"));
        while (true) Thread.Sleep(1);
    }
}
