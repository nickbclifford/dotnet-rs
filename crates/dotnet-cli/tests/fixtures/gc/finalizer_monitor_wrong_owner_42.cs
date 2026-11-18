using System;
using System.Runtime.CompilerServices;
using System.Threading;

class Payload {
    public static readonly object Lock = new object();
    public static bool Threw;
    ~Payload() {
        try { Monitor.Exit(Lock); }
        catch (SynchronizationLockException) { Threw = true; }
    }
}
class Program {
    [MethodImpl(MethodImplOptions.NoInlining)] static void Make() { new Payload(); }
    static int Main() {
        lock (Payload.Lock) {
            Make();
            GC.Collect();
            GC.WaitForPendingFinalizers();
            return Payload.Threw && Monitor.IsEntered(Payload.Lock) ? 42 : 1;
        }
    }
}
