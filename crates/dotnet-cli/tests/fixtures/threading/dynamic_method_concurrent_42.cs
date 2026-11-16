using System;
using System.Reflection.Emit;
using System.Threading;

class Program
{
    private static int ready;
    private static volatile bool go;
    private static readonly int[] results = new int[4];
    private static readonly Exception[] errors = new Exception[4];

    static void Run(int index)
    {
        try
        {
            Interlocked.Increment(ref ready);
            while (!go) Thread.Yield();
            var method = new DynamicMethod("Concurrent" + index, typeof(int), Type.EmptyTypes);
            var il = method.GetILGenerator();
            il.Emit(OpCodes.Ldc_I4, 39 + index);
            il.Emit(OpCodes.Ret);
            results[index] = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        }
        catch (Exception error) { errors[index] = error; }
    }

    static int Main()
    {
        var threads = new Thread[4];
        for (int i = 0; i < threads.Length; i++)
        {
            int index = i;
            threads[i] = new Thread(() => Run(index));
            threads[i].Start();
        }
        while (Volatile.Read(ref ready) != threads.Length) Thread.Yield();
        go = true;
        foreach (var thread in threads) thread.Join();
        for (int i = 0; i < threads.Length; i++)
            if (errors[i] != null || results[i] != 39 + i) return 1;
        Console.WriteLine("concurrent:ok");
        return 42;
    }
}
