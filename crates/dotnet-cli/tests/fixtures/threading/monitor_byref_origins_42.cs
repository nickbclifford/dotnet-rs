using System.Threading;

public static class Program
{
    static object gate = new object();
    static bool staticTaken;

    sealed class Flag
    {
        public bool taken;
    }

    public static int Main()
    {
        Monitor.TryEnter(gate, 0, ref staticTaken);
        if (!staticTaken) return 1;
        Monitor.Exit(gate);

        var flag = new Flag();
        Monitor.Enter(gate, ref flag.taken);
        if (!flag.taken) return 2;
        Monitor.Exit(gate);
        return 42;
    }
}
