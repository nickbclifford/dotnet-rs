using System;
using System.Threading;

class Payload
{
    public int Value;
}

class Program
{
    static Payload? Saved;

    static void Create()
    {
        Saved = new Payload { Value = 42 };
    }

    static int Main()
    {
        var worker = new Thread(Create);
        worker.Start();
        worker.Join();

        for (int i = 0; i < 3; i++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
            if (Saved == null || Saved.Value != 42)
                return 1;
        }
        return 42;
    }
}
