using System;
using System.Runtime.CompilerServices;
using System.Threading;

class Payload
{
    ~Payload() { Thread.Sleep(2); }
}

class Program
{
    static volatile bool stop;
    static volatile bool producing;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Create() { new Payload(); }

    static void Produce()
    {
        producing = true;
        while (!stop)
        {
            Create();
            GC.Collect();
            Thread.Sleep(5);
        }
    }

    static int Main()
    {
        Create();
        GC.Collect();
        var producer = new Thread(Produce);
        producer.Start();
        while (!producing) Thread.Yield();
        var watchdog = new Thread(() => { Thread.Sleep(5000); stop = true; });
        watchdog.IsBackground = true;
        watchdog.Start();
        GC.WaitForPendingFinalizers();
        bool returnedWhileProducing = !stop;
        stop = true;
        producer.Join();
        return returnedWhileProducing ? 42 : 1;
    }
}
