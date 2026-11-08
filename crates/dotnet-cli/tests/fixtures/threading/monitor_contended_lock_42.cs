using System.Threading;

public static class Program
{
    static object gate = new object();
    static volatile bool held;
    static volatile bool attempting;
    static int value;

    static void Owner()
    {
        lock (gate)
        {
            held = true;
            while (!attempting) Thread.Sleep(1);
            Thread.Sleep(30);
            value = 40;
        }
    }

    public static int Main()
    {
        // Keep the shared gate in the main arena while the worker can retire.
        object mainGate = gate;
        var owner = new Thread(Owner);
        owner.Start();
        while (!held) Thread.Sleep(1);
        attempting = true;
        lock (mainGate)
        {
            value += 2;
            lock (mainGate) { value += 0; }
        }
        owner.Join();
        return value == 42 ? 42 : 1;
    }
}
