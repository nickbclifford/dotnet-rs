using System;
using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        object gate = new object();
        bool ready = false, done = false;
        Task<int> task;
        bool mainIsClear;
        lock (gate)
        {
            task = Task.Run(() => {
                lock (gate) {
                    ready = true;
                    Monitor.PulseAll(gate);
                    while (!done) Monitor.Wait(gate);
                }
                return Task.CurrentId.HasValue ? 42 : 1;
            });
            while (!ready) Monitor.Wait(gate);
            mainIsClear = Task.CurrentId == null;
            done = true;
            Monitor.PulseAll(gate);
        }
        Console.WriteLine($"main-clear={mainIsClear}");
        return task.Result == 42 && mainIsClear ? 42 : 1;
    }
}
