using System;
using System.Threading;

class Payload
{
    ~Payload()
    {
        Program.FinalizerStarted = true;
        lock (Program.Gate) { Program.Done = true; }
    }
}

class Program
{
    public static object Gate = new object();
    public static volatile bool Locked;
    public static volatile bool FinalizerStarted;
    public static bool Done;
    static void Create() { new Payload(); }
    static void Hold()
    {
        lock (Gate)
        {
            Locked = true;
            while (!FinalizerStarted) { }
            Thread.Sleep(300);
        }
    }
    static int Main()
    {
        var creator = new Thread(Create);
        creator.Start();
        creator.Join();
        var holder = new Thread(Hold);
        holder.Start();
        while (!Locked) { }
        GC.Collect();
        while (!FinalizerStarted) { }
        GC.WaitForPendingFinalizers();
        bool observed = Done;
        holder.Join();
        Console.WriteLine(observed);
        return observed ? 42 : 1;
    }
}
