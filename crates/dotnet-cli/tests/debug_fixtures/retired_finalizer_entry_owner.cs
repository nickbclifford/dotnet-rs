using System;
using System.Threading;

class Payload
{
    ~Payload()
    {
        Program.Started = 1;
        Thread.Sleep(150);
        GC.Collect();
        lock (Program.Gate) { Program.Done = 1; }
    }
}

class Program
{
    public static object Gate = new object();
    public static volatile int Started;
    public static int Done;
    static void Create() { new Payload(); }
    static int Main()
    {
        var gate = Gate;
        var worker = new Thread(Create);
        worker.Start();
        worker.Join();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return 42;
    }
}
