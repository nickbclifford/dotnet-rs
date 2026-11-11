using System;
using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        for (int i = 0; i < 200; i++) {
            int calls = 0;
            Task<int> task = Task.Run(() => {
                Interlocked.Increment(ref calls);
                return 42;
            });
            if (task.Result != 42 || calls != 1) return 1;
        }
        Console.WriteLine("once=True");
        return 42;
    }
}
