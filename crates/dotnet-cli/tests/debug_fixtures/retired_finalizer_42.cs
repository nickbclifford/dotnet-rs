using System;
using System.Threading;

class Payload
{
    ~Payload() { Program.Done = true; }
}

class Program
{
    public static bool Done;
    static void Create() { new Payload(); }
    static int Main()
    {
        var worker = new Thread(Create);
        worker.Start();
        worker.Join();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        Console.WriteLine(Done);
        return Done ? 42 : 1;
    }
}
