using System;
using System.Threading;

public static class Program
{
    static void Worker() { throw new InvalidOperationException("worker failure"); }
    public static int Main()
    {
        Console.WriteLine("before worker");
        var worker = new Thread(Worker);
        worker.Start();
        try { worker.Join(); }
        catch (InvalidOperationException) { Console.WriteLine("caught through join"); return 42; }
        Console.WriteLine("joined");
        return 42;
    }
}
