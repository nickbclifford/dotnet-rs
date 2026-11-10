using System;
using System.Threading;

public static class Program
{
    static readonly object Gate = new object();
    static int ready;

    static void Worker()
    {
        Volatile.Write(ref ready, 1);
        lock (Gate) { while (true) Thread.Sleep(50); }
    }

    public static void Main()
    {
        lock (Gate)
        {
            new Thread(Worker).Start();
            while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
            Console.WriteLine("before entry failure");
            throw new InvalidOperationException("entry failure");
        }
    }
}
