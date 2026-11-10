using System;
using System.Threading;

class Program
{
    static void Complete()
    {
        // Keep the worker pending long enough for Join to take its cooperative-yield path.
        Thread.Sleep(10);
    }

    static int Main()
    {
        var completed = new Thread(Complete);
        completed.Start();
        try
        {
            completed.Start();
            return 1;
        }
        catch (ThreadStateException)
        {
        }
        completed.Join();

        return 42;
    }
}
