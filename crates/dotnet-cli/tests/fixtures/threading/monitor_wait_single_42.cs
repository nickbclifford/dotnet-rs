using System;
using System.Threading;

public static class Program
{
    public static int Main()
    {
        object gate = new object();
        Monitor.Enter(gate);
        Monitor.Exit(gate);
        try { Monitor.Wait(gate); return 7; } catch (SynchronizationLockException) { }
        try { Monitor.Wait(gate, 0); return 1; } catch (SynchronizationLockException) { }
        try { Monitor.Pulse(gate); return 2; } catch (SynchronizationLockException) { }
        try { Monitor.PulseAll(gate); return 3; } catch (SynchronizationLockException) { }
        try { Monitor.Wait(null, 0); return 4; } catch (ArgumentNullException) { }
        try { Monitor.Wait(gate, -2); return 5; } catch (ArgumentOutOfRangeException) { }
        lock (gate)
        {
            Monitor.Pulse(gate); // A pulse without a waiter is lost.
            lock (gate)
            {
                if (Monitor.Wait(gate, 10)) return 6;
            }
            Monitor.PulseAll(gate); // The outer lock must still be held.
        }
        return 42;
    }
}
