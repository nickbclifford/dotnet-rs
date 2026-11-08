using System;
using System.Threading;

class Payload
{
    ~Payload() { Program.Done = true; }
}

class Program
{
    public static bool Done;
    static void Allocate() { new Payload(); }
    static void Worker()
    {
        Thread.Sleep(200);
        GC.Collect();
        GC.WaitForPendingFinalizers();
        Console.WriteLine(Done);
    }
    static int Main()
    {
        Allocate();
        new Thread(Worker).Start();
        return 42;
    }
}
