using System;
using System.Threading;

public static class Program
{
    static void Worker() { Thread.Sleep(100); throw new InvalidOperationException("worker failure"); }
    public static int Main()
    {
        Console.WriteLine("before worker");
        new Thread(Worker).Start();
        return 42;
    }
}
