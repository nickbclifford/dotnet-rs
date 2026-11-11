using System;
using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeAsync()
    {
        int value = await Task.FromResult(40);
        if (value != 40)
        {
            return -1;
        }

        throw new InvalidOperationException("boom");
    }

    private static async Task<bool> CheckAwait(Task<int> task)
    {
        try { await task; return false; }
        catch (InvalidOperationException ex) { return ex.Message == "boom"; }
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
            if (ex.InnerException is not InvalidOperationException inner || inner.Message != "boom")
                return 2;
        }
        return CheckAwait(task).Result ? 42 : 3;
    }
}
