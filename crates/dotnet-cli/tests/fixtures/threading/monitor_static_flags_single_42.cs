using System.Threading;

public static class Program
{
    static readonly object gate = new object();
    static bool enterTaken, timedTaken;

    public static int Main()
    {
        Monitor.Enter(gate, ref enterTaken);
        if (!enterTaken) return 1;
        Monitor.Exit(gate);
        GC.Collect();
        Monitor.TryEnter(gate, 300, ref timedTaken);
        if (!timedTaken) return 2;
        Monitor.Exit(gate);
        return 42;
    }
}
