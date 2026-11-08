using System;
using System.Threading;

class Payload
{
    ~Payload() { Program.Done = true; }
}

class Program
{
    public static volatile bool Ready;
    public static volatile bool Release;
    public static bool Done;
    static void Allocate() { new Payload(); }
    static void Worker()
    {
        Allocate();
        Ready = true;
        while (!Release) { }
    }
    static int Main()
    {
        var worker = new Thread(Worker);
        worker.Start();
        while (!Ready) { }
        GC.Collect();
        GC.WaitForPendingFinalizers();
        bool observed = Done;
        Release = true;
        worker.Join();
        Console.WriteLine(observed);
        return observed ? 42 : 1;
    }
}
