using System;
using System.Runtime.CompilerServices;

class Payload {
    [ThreadStatic] public static int Value;
    public static int Seen;
    public static int Ran;
    ~Payload() { Seen = Value; Value = 27; Ran++; }
}
class Program {
    [MethodImpl(MethodImplOptions.NoInlining)] static void Make() { new Payload(); }
    static int Main() {
        Payload.Value = 11;
        Make();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return Payload.Ran == 1 && Payload.Seen == 0 && Payload.Value == 11 ? 42 : 1;
    }
}
