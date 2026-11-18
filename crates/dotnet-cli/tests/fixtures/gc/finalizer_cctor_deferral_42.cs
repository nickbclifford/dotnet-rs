using System;
using System.Runtime.CompilerServices;

class Payload {
    public static bool Observed;
    ~Payload() { Observed = Initializing.Value == 42; }
}
class Initializing {
    public static int Value;
    [MethodImpl(MethodImplOptions.NoInlining)] static void Create() { new Payload(); }
    static Initializing() { Create(); GC.Collect(); Value = 42; }
}
class Program {
    static int Main() {
        int value = Initializing.Value;
        GC.WaitForPendingFinalizers();
        return value == 42 && Payload.Observed ? 42 : 1;
    }
}
