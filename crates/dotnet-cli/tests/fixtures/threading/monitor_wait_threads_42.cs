using System;
using System.Threading;

public static class Program
{
    static readonly object gate = new object();
    static int ready, woken, payload;
    static bool done;
    static void Worker()
    {
        lock (gate)
        {
            lock (gate)
            {
                ready++;
                Monitor.PulseAll(gate);
                while (!done)
                    if (!Monitor.Wait(gate, 5000)) return;
                if (payload == 42) woken++;
            }
            Monitor.PulseAll(gate); // Wait restored both recursion levels.
        }
    }
    public static int Main()
    {
        Thread a = new Thread(Worker), b = new Thread(Worker), c = new Thread(Worker);
        a.Start(); b.Start(); c.Start();
        lock (gate)
        {
            while (ready != 3)
                if (!Monitor.Wait(gate, 5000)) return 1;
            payload = 42;
            done = true;
            Monitor.PulseAll(gate);
        }
        a.Join(); b.Join(); c.Join();
        if (woken != 3) return 2;
        bool pulsed = false;
        Thread producer = new Thread(() => { lock (gate) { pulsed = true; Monitor.Pulse(gate); } });
        lock (gate)
        {
            producer.Start();
            while (!pulsed)
                Monitor.Wait(gate);
        }
        producer.Join();
        return 42;
    }
}
