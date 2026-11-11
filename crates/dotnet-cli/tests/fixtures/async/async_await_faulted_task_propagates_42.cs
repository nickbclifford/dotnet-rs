using System;
using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeAsync()
    {
        await Task.FromException(new ApplicationException("faulted-task"));
        return 0;
    }

    private static async Task<bool> CheckAwait(Task<int> task)
    {
        try { await task; return false; }
        catch (ApplicationException ex) { return ex.Message == "faulted-task"; }
    }

    public static int Main()
    {
        Task<int> task = ComputeAsync();
        try
        {
            _ = task.Result;
            return 1;
        }
        catch (AggregateException ex)
        {
            if (ex.InnerException is not ApplicationException inner || inner.Message != "faulted-task")
                return 2;
        }
        return CheckAwait(task).Result ? 42 : 3;
    }
}
