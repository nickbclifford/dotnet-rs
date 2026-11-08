using System;
using System.Threading;

class Payload
{
    ~Payload()
    {
        Program.Started = 1;
        while (true) { }
    }
}

class Program
{
    public static volatile int Started;
    static void Create() { new Payload(); }
    static int Main()
    {
        var worker = new Thread(Create);
        worker.Start();
        worker.Join();
        GC.Collect();
        while (Started == 0) { }
        GC.WaitForPendingFinalizers();
        return 42;
    }
}
