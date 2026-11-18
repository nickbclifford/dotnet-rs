// DOTNET_RS_VM_ONLY: the CLR blocks indefinitely on this program.
using System;
using System.Runtime.CompilerServices;

class Payload {
    public static readonly object Gate = new object();
    ~Payload() { lock (Gate) { } }
}
class Program {
    [MethodImpl(MethodImplOptions.NoInlining)] static void Create() { new Payload(); }
    static int Main() {
        lock (Payload.Gate) { Create(); GC.Collect(); GC.WaitForPendingFinalizers(); }
        return 0;
    }
}
