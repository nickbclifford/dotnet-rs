using System;
using System.Threading;

class Program
{
    static void Complete()
    {
        // Keep the worker pending long enough for Join to take its cooperative-yield path.
        Thread.Sleep(10);
    }

    static void Fail()
    {
        throw new Exception("expected managed worker failure");
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
        catch (InvalidOperationException)
        {
        }
        completed.Join();

        var failing = new Thread(Fail);
        failing.Start();
        try
        {
            failing.Join();
            return 2;
        }
        catch (InvalidOperationException)
        {
            return 42;
        }
    }
}
