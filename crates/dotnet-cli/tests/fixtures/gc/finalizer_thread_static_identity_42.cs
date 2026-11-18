using System;
using System.Runtime.CompilerServices;

class Payload {
    [ThreadStatic] public static int Value;
    public static int Seen;
    ~Payload() { Seen = Value; Value = 27; }
}
class Program {
    [MethodImpl(MethodImplOptions.NoInlining)] static void Make() { new Payload(); }
    static int Main() {
        Payload.Value = 11;
        Make();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return Payload.Seen == 0 && Payload.Value == 11 ? 42 : 1;
    }
}
