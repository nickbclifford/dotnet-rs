using System;
using System.Threading;

class Payload
{
    ~Payload()
    {
        Program.Started = true;
        lock (Program.Gate) { Program.Done = true; }
    }
}

class Program
{
    public static object Gate = new object();
    public static volatile bool Started;
    public static bool Done;
    static void Create() { new Payload(); }
    static int Main()
    {
        Monitor.Enter(Gate);
        var worker = new Thread(Create);
        worker.Start();
        worker.Join();
        GC.Collect();
        while (!Started) { }
        Monitor.Exit(Gate);
        GC.WaitForPendingFinalizers();
        Console.WriteLine(Done);
        return Done ? 42 : 1;
    }
}
