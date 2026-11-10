using System;
using System.Threading;

public static class Program
{
    static void Worker() { throw new InvalidOperationException("worker failure"); }
    public static int Main()
    {
        Console.WriteLine("before worker");
        new Thread(Worker).Start();
        while (true) Thread.Sleep(1);
    }
}
