using System;
using System.Threading;

public static class Program
{
    static object gate = new object();

    static void Worker()
    {
        Thread.Sleep(100);
        GC.Collect();
        Console.WriteLine(gate != null ? "worker completed" : "missing gate");
    }

    public static int Main()
    {
        object mainGate = gate;
        var worker = new Thread(Worker);
        worker.Start();
        return mainGate != null ? 42 : 1;
    }
}
