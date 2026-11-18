using System;
using System.Runtime.CompilerServices;
using System.Threading;

class Payload {
    public static readonly object Gate = new object();
    public static bool Released;
    public static bool Observed;
    ~Payload() { lock (Gate) { Observed = Released; } }
}
class Program {
    [MethodImpl(MethodImplOptions.NoInlining)] static void Create() { new Payload(); }
    static int Main() {
        Monitor.Enter(Payload.Gate);
        try { Create(); GC.Collect(); Payload.Released = true; }
        finally { Monitor.Exit(Payload.Gate); }
        GC.WaitForPendingFinalizers();
        return Payload.Observed ? 42 : 1;
    }
}
