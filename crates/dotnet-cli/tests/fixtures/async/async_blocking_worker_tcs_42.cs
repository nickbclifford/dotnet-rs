using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        var source = new TaskCompletionSource<int>();
        object gate = new object();
        bool ready = false;
        var worker = new Thread(() =>
        {
            lock (gate) { ready = true; Monitor.Pulse(gate); }
            Thread.Sleep(200);
            source.SetResult(42);
        });
        lock (gate)
        {
            worker.Start();
            while (!ready) Monitor.Wait(gate);
        }
        int result = source.Task.Result;
        worker.Join();
        return result;
    }
}
