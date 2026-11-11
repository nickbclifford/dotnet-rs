using System.Threading;

public static class Program
{
    public static int Main()
    {
        object gate = new object();
        int completed = 0;
        bool valid = true;
        WaitCallback callback = _ =>
        {
            lock (gate)
            {
                Thread current = Thread.CurrentThread;
                valid &= current.IsThreadPoolThread && current.IsBackground
                    && object.ReferenceEquals(current, Thread.CurrentThread)
                    && current.ManagedThreadId == Thread.CurrentThread.ManagedThreadId;
                completed++;
                Monitor.PulseAll(gate);
            }
        };
        lock (gate)
        {
            if (!ThreadPool.QueueUserWorkItem(callback) || !ThreadPool.UnsafeQueueUserWorkItem(callback, null)) return 1;
            while (completed != 2) Monitor.Wait(gate);
        }
        return valid ? 42 : 1;
    }
}
