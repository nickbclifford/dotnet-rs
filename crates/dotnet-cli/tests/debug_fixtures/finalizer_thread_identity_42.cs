using System;
using System.Runtime.CompilerServices;
using System.Threading;

class Payload
{
    public static Thread? FirstThread;
    public static Thread? SecondThread;
    public static int Count;

    ~Payload()
    {
        if (++Count == 1)
        {
            FirstThread = Thread.CurrentThread;
            GC.ReRegisterForFinalize(this);
        }
        else
        {
            SecondThread = Thread.CurrentThread;
        }
    }
}

class Program
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Create() { new Payload(); }

    static int Main()
    {
        Thread allocatingThread = Thread.CurrentThread;
        Create();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        return Payload.Count == 2 && Payload.FirstThread == Payload.SecondThread
            && Payload.FirstThread != allocatingThread ? 42 : 1;
    }
}
