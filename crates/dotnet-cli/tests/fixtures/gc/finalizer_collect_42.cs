using System;

class Payload
{
    ~Payload() { GC.Collect(); Program.Done = true; }
}

class Program
{
    public static bool Done;
    static void Create() { new Payload(); }
    static int Main()
    {
        Create();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return Done ? 42 : 1;
    }
}
